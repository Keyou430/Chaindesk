//go:build !android

package engine

import (
	"bufio"
	"context"
	"crypto/rand"
	"encoding/base64"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"net"
	"net/http"
	"net/http/httptest"
	"os"
	"strconv"
	"strings"
	"sync/atomic"
	"testing"
	"time"

	"github.com/metacubex/mihomo/component/auth"
	"github.com/metacubex/mihomo/transport/socks5"
	"github.com/metacubex/sing-shadowsocks/shadowaead"
	vmess "github.com/metacubex/sing-vmess"
	M "github.com/metacubex/sing/common/metadata"
)

// All listeners share host loopback; this lab does not prove network isolation.
func TestServeProductEmulatorFixture(t *testing.T) {
	path := os.Getenv("CHAINDESK_PRODUCT_FIXTURE")
	if path == "" {
		t.Skip("product emulator fixture was not requested")
	}
	proxy := os.Getenv("CHAINDESK_PRODUCT_HTTP_PROXY")
	if proxy != "" && proxy != "127.0.0.1:7897" {
		t.Fatal("only the existing local test transport is allowed")
	}
	var targetHits, exitHits, healthHits, rejectedAuth, frontHits, frontErrors, udpHits atomic.Int32
	target := httptest.NewTLSServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if r.URL.Path != "/a0" || len(r.URL.Query().Get("nonce")) != 32 {
			http.NotFound(w, r)
			return
		}
		targetHits.Add(1)
		w.Header().Set("Cache-Control", "no-store")
		io.WriteString(w, "a0-chain-ok")
	}))
	t.Cleanup(target.Close)
	targetPort := target.Listener.Addr().(*net.TCPAddr).Port
	// A non-private documentation address avoids the declared LAN DIRECT rules.
	expectedTarget := net.JoinHostPort("198.51.100.1", strconv.Itoa(targetPort))
	udp, err := net.ListenPacket("udp4", "127.0.0.1:0")
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { udp.Close() })
	go func() {
		buffer := make([]byte, 256)
		for {
			n, _, err := udp.ReadFrom(buffer)
			if err != nil {
				return
			}
			if n == len("chaindesk-udp:")+32 && strings.HasPrefix(string(buffer[:n]), "chaindesk-udp:") {
				udpHits.Add(1)
			}
		}
	}()
	password := rand.Text()
	exit := listenFixture(t, func(connection net.Conn) {
		connection.SetDeadline(time.Now().Add(30 * time.Second))
		address, command, _, err := socks5.ServerHandshake(connection, auth.NewAuthenticator([]auth.AuthUser{{User: "product-test", Pass: password}}))
		if err != nil {
			if errors.Is(err, socks5.ErrAuth) {
				rejectedAuth.Add(1)
			}
			return
		}
		if command != socks5.CmdConnect {
			return
		}
		var remote net.Conn
		if address.String() == expectedTarget {
			exitHits.Add(1)
			remote, err = net.DialTimeout("tcp4", target.Listener.Addr().String(), time.Second)
		} else if productHealthAuthority(address.String()) {
			healthHits.Add(1)
			for start := time.Now(); time.Since(start) < 25*time.Second; {
				if _, held := os.Stat(path + ".holdhealth"); held != nil {
					break
				}
				time.Sleep(20 * time.Millisecond)
			}
			remote, err = dialProductHealth(address.String(), proxy)
		} else {
			return
		}
		if err == nil {
			relayFixture(connection, remote)
		}
	})
	frontHandler := &frontFixture{exit: exit.Addr().String()}
	frontPassword := rand.Text()
	frontKind := os.Getenv("CHAINDESK_PRODUCT_FRONT")
	if frontKind == "" {
		frontKind = "ss-aes-128-gcm"
	}
	node := map[string]any{"type": "ss", "server": "10.0.2.2"}
	var accept func(context.Context, net.Conn, M.Metadata) error
	cipher := strings.TrimPrefix(frontKind, "ss-")
	switch frontKind {
	case "ss-aes-128-gcm", "ss-aes-256-gcm", "ss-chacha20-ietf-poly1305":
		service, err := shadowaead.NewService(cipher, nil, frontPassword, 30, frontHandler)
		if err != nil {
			t.Fatal(err)
		}
		accept = service.NewConnection
		node["cipher"], node["password"] = cipher, frontPassword
	case "vmess-tcp":
		identifier := make([]byte, 16)
		if _, err := rand.Read(identifier); err != nil {
			t.Fatal(err)
		}
		identifier[6], identifier[8] = identifier[6]&15|64, identifier[8]&63|128
		uuid := fmt.Sprintf("%x-%x-%x-%x-%x", identifier[:4], identifier[4:6], identifier[6:8], identifier[8:10], identifier[10:])
		service := vmess.NewService[string](frontHandler, vmess.ServiceWithDisableHeaderProtection())
		if err := service.UpdateUsers([]string{"product-test"}, []string{uuid}, []int{0}); err != nil {
			t.Fatal(err)
		}
		if err := service.Start(); err != nil {
			t.Fatal(err)
		}
		t.Cleanup(func() { service.Close() })
		accept = service.NewConnection
		node["type"], node["uuid"], node["alterId"], node["cipher"], node["network"] = "vmess", uuid, 0, "auto", "tcp"
	default:
		t.Fatal("unsupported controlled front")
	}
	front := listenFixture(t, func(connection net.Conn) {
		frontHits.Add(1)
		connection.SetDeadline(time.Now().Add(30 * time.Second))
		if err := accept(context.Background(), connection, M.Metadata{}); err != nil {
			frontErrors.Add(1)
		}
	})
	node["port"] = front.Addr().(*net.TCPAddr).Port
	metadata := map[string]any{
		"fixture": fixtureInput{
			Front: fixtureEndpoint{Server: "10.0.2.2", Port: front.Addr().(*net.TCPAddr).Port, Cipher: cipher, Password: frontPassword},
			Exit:  fixtureEndpoint{Server: "127.0.0.1", Port: exit.Addr().(*net.TCPAddr).Port, Username: "product-test", Password: password},
		},
		"targetHost": "198.51.100.1", "targetPort": targetPort,
		"frontNode": node, "frontKind": frontKind,
		"udpPort":               udp.LocalAddr().(*net.UDPAddr).Port,
		"certificate":           base64.StdEncoding.EncodeToString(target.Certificate().Raw),
		"publicHealthTransport": map[bool]string{true: "existing-loopback-http-connect", false: "host-network"}[proxy != ""],
	}
	encoded, err := json.Marshal(metadata)
	if err != nil {
		t.Fatal(err)
	}
	if err = os.WriteFile(path, encoded, 0600); err != nil {
		t.Fatal(err)
	}
	ticker := time.NewTicker(50 * time.Millisecond)
	defer ticker.Stop()
	for range ticker.C {
		counts, _ := json.Marshal(map[string]int32{
			"frontAccepted": frontHits.Load(), "frontErrors": frontErrors.Load(), "frontConnections": frontHandler.hits.Load(),
			"exitTargetConnections": exitHits.Load(), "targetRequests": targetHits.Load(), "rejectedAuthentication": rejectedAuth.Load(),
			"publicHealthConnections": healthHits.Load(), "udpPackets": udpHits.Load(),
		})
		if err = os.WriteFile(path+".counts", counts, 0600); err != nil {
			t.Fatal(err)
		}
		if _, err = os.Stat(path + ".stop"); err == nil {
			return
		}
	}
}

func productHealthAuthority(address string) bool {
	switch address {
	case "www.gstatic.com:443", "cp.cloudflare.com:443", "api.ipify.org:443", "www.cloudflare.com:443", "1.1.1.1:443":
		return true
	}
	return false
}

func dialProductHealth(address, proxy string) (net.Conn, error) {
	if proxy == "" {
		return net.DialTimeout("tcp4", address, 5*time.Second)
	}
	connection, err := net.DialTimeout("tcp4", proxy, time.Second)
	if err != nil {
		return nil, err
	}
	connection.SetDeadline(time.Now().Add(5 * time.Second))
	if _, err = fmt.Fprintf(connection, "CONNECT %s HTTP/1.1\r\nHost: %s\r\n\r\n", address, address); err == nil {
		var response *http.Response
		response, err = http.ReadResponse(bufio.NewReader(connection), &http.Request{Method: http.MethodConnect})
		if err == nil && response.StatusCode != http.StatusOK {
			err = errors.New("health transport refused CONNECT")
		}
	}
	if err != nil {
		connection.Close()
		return nil, err
	}
	connection.SetDeadline(time.Time{})
	return connection, nil
}

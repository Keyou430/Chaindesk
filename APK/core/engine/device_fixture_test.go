//go:build !android

package engine

import (
	"context"
	"crypto/rand"
	"encoding/base64"
	"encoding/json"
	"errors"
	"io"
	"net"
	"net/http"
	"net/http/httptest"
	"os"
	"strconv"
	"sync/atomic"
	"testing"
	"time"

	"github.com/metacubex/mihomo/component/auth"
	"github.com/metacubex/mihomo/transport/socks5"
	"github.com/metacubex/sing-shadowsocks/shadowaead"
	M "github.com/metacubex/sing/common/metadata"
)

// This opt-in lab shares the host's loopback network; it is not device isolation.
func TestServeA0EmulatorFixture(t *testing.T) {
	metadataPath := os.Getenv("CHAINDESK_A0_FIXTURE")
	if metadataPath == "" {
		t.Skip("emulator fixture was not requested")
	}
	var targetHits, exitHits, rejectedAuth, frontAccepted, frontErrors atomic.Int32
	target := httptest.NewTLSServer(http.HandlerFunc(func(w http.ResponseWriter, request *http.Request) {
		if request.URL.Path != "/a0" || request.URL.Query().Get("nonce") == "" {
			http.NotFound(w, request)
			return
		}
		targetHits.Add(1)
		w.Header().Set("Cache-Control", "no-store")
		io.WriteString(w, "a0-chain-ok")
	}))
	t.Cleanup(target.Close)
	targetPort := target.Listener.Addr().(*net.TCPAddr).Port
	expectedTarget := net.JoinHostPort("10.0.2.2", strconv.Itoa(targetPort))
	password := rand.Text()
	exit := listenFixture(t, func(connection net.Conn) {
		connection.SetDeadline(time.Now().Add(30 * time.Second))
		address, command, _, err := socks5.ServerHandshake(connection, auth.NewAuthenticator([]auth.AuthUser{{User: "a0-test", Pass: password}}))
		if err != nil {
			if errors.Is(err, socks5.ErrAuth) {
				rejectedAuth.Add(1)
			}
			return
		}
		if command != socks5.CmdConnect || address.String() != expectedTarget {
			return
		}
		exitHits.Add(1)
		// 10.0.2.2 is the emulator's host-loopback alias, not a host interface.
		remote, err := net.DialTimeout("tcp4", target.Listener.Addr().String(), time.Second)
		if err == nil {
			relayFixture(connection, remote)
		}
	})
	frontHandler := &frontFixture{exit: exit.Addr().String()}
	frontPassword := rand.Text()
	service, err := shadowaead.NewService("aes-128-gcm", nil, frontPassword, 30, frontHandler)
	if err != nil {
		t.Fatal(err)
	}
	front := listenFixture(t, func(connection net.Conn) {
		frontAccepted.Add(1)
		connection.SetDeadline(time.Now().Add(30 * time.Second))
		if err := service.NewConnection(context.Background(), connection, M.Metadata{}); err != nil {
			frontErrors.Add(1)
		}
	})
	metadata := struct {
		Fixture     fixtureInput `json:"fixture"`
		TargetPort  int          `json:"targetPort"`
		Certificate string       `json:"certificate"`
	}{
		Fixture: fixtureInput{
			Front: fixtureEndpoint{Server: "10.0.2.2", Port: front.Addr().(*net.TCPAddr).Port, Cipher: "aes-128-gcm", Password: frontPassword},
			Exit:  fixtureEndpoint{Server: "127.0.0.1", Port: exit.Addr().(*net.TCPAddr).Port, Username: "a0-test", Password: password},
		},
		TargetPort:  targetPort,
		Certificate: base64.StdEncoding.EncodeToString(target.Certificate().Raw),
	}
	encoded, err := json.Marshal(metadata)
	if err != nil {
		t.Fatal(err)
	}
	if err = os.WriteFile(metadataPath, encoded, 0600); err != nil {
		t.Fatal(err)
	}
	ticker := time.NewTicker(100 * time.Millisecond)
	defer ticker.Stop()
	for range ticker.C {
		counts, _ := json.Marshal(map[string]int32{"frontAccepted": frontAccepted.Load(), "frontErrors": frontErrors.Load(), "frontConnections": frontHandler.hits.Load(), "exitTargetConnections": exitHits.Load(), "targetRequests": targetHits.Load(), "rejectedAuthentication": rejectedAuth.Load()})
		if err = os.WriteFile(metadataPath+".counts", counts, 0600); err != nil {
			t.Fatal(err)
		}
		if _, err = os.Stat(metadataPath + ".stop"); err == nil {
			return
		}
	}
}

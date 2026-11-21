package engine

import (
	"context"
	"crypto/tls"
	"crypto/x509"
	"encoding/json"
	"errors"
	"io"
	"net"
	"net/http"
	"net/http/httptest"
	"net/netip"
	"strconv"
	"sync"
	"sync/atomic"
	"testing"
	"time"

	"github.com/metacubex/mihomo/component/auth"
	C "github.com/metacubex/mihomo/constant"
	"github.com/metacubex/mihomo/listener/sing"
	"github.com/metacubex/mihomo/transport/socks5"
	"github.com/metacubex/mihomo/tunnel"
	"github.com/metacubex/sing-shadowsocks/shadowaead"
	M "github.com/metacubex/sing/common/metadata"
	N "github.com/metacubex/sing/common/network"
)

type frontFixture struct {
	exit string
	hits atomic.Int32
}

func (f *frontFixture) NewConnection(ctx context.Context, connection net.Conn, metadata M.Metadata) error {
	if metadata.Destination.String() != f.exit {
		return errors.New("front may only reach exit")
	}
	f.hits.Add(1)
	remote, err := (&net.Dialer{}).DialContext(ctx, "tcp4", f.exit)
	if err != nil {
		return err
	}
	relayFixture(connection, remote)
	return nil
}
func (*frontFixture) NewPacketConnection(context.Context, N.PacketConn, M.Metadata) error {
	return errors.New("UDP forbidden")
}
func (*frontFixture) NewError(context.Context, error) {}

func relayFixture(left, right net.Conn) {
	defer right.Close()
	done := make(chan struct{})
	go func() { io.Copy(right, left); right.Close(); close(done) }()
	io.Copy(left, right)
	left.Close()
	<-done
}

func listenFixture(t *testing.T, serve func(net.Conn)) net.Listener {
	t.Helper()
	listener, err := net.Listen("tcp4", "127.0.0.1:0")
	if err != nil {
		t.Fatal(err)
	}
	var workers sync.WaitGroup
	workers.Add(1)
	go func() {
		defer workers.Done()
		for {
			connection, err := listener.Accept()
			if err != nil {
				return
			}
			workers.Add(1)
			go func() {
				defer workers.Done()
				defer connection.Close()
				connection.SetDeadline(time.Now().Add(3 * time.Second))
				serve(connection)
			}()
		}
	}()
	t.Cleanup(func() { listener.Close(); workers.Wait() })
	return listener
}

func TestRealSSAndAuthenticatedSOCKSChainRejectsBypassAndFailures(t *testing.T) {
	var targetHits, exitHits atomic.Int32
	target := httptest.NewTLSServer(http.HandlerFunc(func(w http.ResponseWriter, _ *http.Request) {
		targetHits.Add(1)
		io.WriteString(w, "chain-ok")
	}))
	t.Cleanup(target.Close)
	targetAddress := target.Listener.Addr().String()
	exit := listenFixture(t, func(connection net.Conn) {
		address, command, _, err := socks5.ServerHandshake(connection, auth.NewAuthenticator([]auth.AuthUser{{User: "fixture-user", Pass: "fixture-exit-secret"}}))
		if err != nil || command != socks5.CmdConnect || address.String() != targetAddress {
			return
		}
		exitHits.Add(1)
		remote, err := net.DialTimeout("tcp4", targetAddress, time.Second)
		if err == nil {
			relayFixture(connection, remote)
		}
	})
	frontHandler := &frontFixture{exit: exit.Addr().String()}
	service, err := shadowaead.NewService("aes-128-gcm", nil, "fixture-front-secret", 30, frontHandler)
	if err != nil {
		t.Fatal(err)
	}
	front := listenFixture(t, func(connection net.Conn) { service.NewConnection(context.Background(), connection, M.Metadata{}) })
	frontPort := front.Addr().(*net.TCPAddr).Port
	exitPort := exit.Addr().(*net.TCPAddr).Port
	input := fixtureInput{
		Front: fixtureEndpoint{Server: "127.0.0.1", Port: frontPort, Cipher: "aes-128-gcm", Password: "fixture-front-secret"},
		Exit:  fixtureEndpoint{Server: "127.0.0.1", Port: exitPort, Username: "fixture-user", Password: "fixture-exit-secret"},
	}
	rootCAs := x509.NewCertPool()
	rootCAs.AddCert(target.Certificate())
	for _, scenario := range []string{"healthy", "healthy-tun-handler", "paused-direct-probe", "paused-tun-reject", "protect-denied", "wrong-exit-auth", "exit-down", "front-down"} {
		t.Run(scenario, func(t *testing.T) {
			candidate := input
			if scenario == "wrong-exit-auth" {
				candidate.Exit.Password = "wrong"
			}
			if scenario == "exit-down" {
				exit.Close()
			}
			if scenario == "front-down" {
				front.Close()
			}
			encoded, _ := json.Marshal(candidate)
			id, err := startSession(encoded, ownedPipe(t), func() (io.Closer, error) { return &countedTunnel{}, nil }, func(fd int) bool {
				if scenario == "protect-denied" {
					return false
				}
				return true
			})
			if err != nil {
				t.Fatal(err)
			}
			t.Cleanup(func() { StopSession(id) })
			BlockSockets(id - 1)
			if scenario == "paused-direct-probe" || scenario == "paused-tun-reject" {
				tunnel.OnSuspend()
			}
			transport := &http.Transport{
				DisableKeepAlives: true,
				TLSClientConfig:   &tls.Config{RootCAs: rootCAs, MinVersion: tls.VersionTLS12},
				DialContext: func(ctx context.Context, _, address string) (net.Conn, error) {
					host, port, err := net.SplitHostPort(address)
					if err != nil {
						return nil, err
					}
					value, _ := strconv.ParseUint(port, 10, 16)
					if scenario == "healthy-tun-handler" || scenario == "paused-tun-reject" {
						handler, err := sing.NewListenerHandler(sing.ListenerConfig{Tunnel: tunnel.Tunnel, Type: C.TUN})
						if err != nil {
							return nil, err
						}
						client, server := net.Pipe()
						go handler.NewConnection(ctx, server, M.Metadata{Source: M.ParseSocksaddr("127.0.0.1:12345"), Destination: M.ParseSocksaddr(address)})
						return client, nil
					}
					return tunnel.Proxies()["CHAIN-EXIT"].DialContext(ctx, &C.Metadata{NetWork: C.TCP, DstIP: netip.MustParseAddr(host), DstPort: uint16(value)})
				},
			}
			defer transport.CloseIdleConnections()
			before := targetHits.Load()
			frontBefore := frontHandler.hits.Load()
			exitBefore := exitHits.Load()
			response, err := (&http.Client{Transport: transport, Timeout: time.Second}).Get(target.URL)
			if scenario == "healthy" || scenario == "healthy-tun-handler" || scenario == "paused-direct-probe" {
				if err != nil {
					t.Fatal(err)
				}
				body, readErr := io.ReadAll(response.Body)
				response.Body.Close()
				if readErr != nil || string(body) != "chain-ok" || frontHandler.hits.Load() != frontBefore+1 || exitHits.Load() != exitBefore+1 || targetHits.Load() != before+1 {
					t.Fatal("both authenticated hops did not carry HTTPS")
				}
			} else {
				if response != nil {
					response.Body.Close()
				}
				if err == nil || targetHits.Load() != before {
					t.Fatal("failed chain reached target")
				}
			}
			if scenario == "protect-denied" && frontHandler.hits.Load() != frontBefore {
				t.Fatal("denied protection still sent data")
			}
			StopSession(id)
		})
	}
}

func TestStopCancelsPendingSOCKSHandshakeBeforeReturning(t *testing.T) {
	entered := make(chan struct{})
	exit := listenFixture(t, func(connection net.Conn) {
		greeting := make([]byte, 3)
		if _, err := io.ReadFull(connection, greeting); err == nil {
			close(entered)
			io.Copy(io.Discard, connection)
		}
	})
	frontHandler := &frontFixture{exit: exit.Addr().String()}
	service, err := shadowaead.NewService("aes-128-gcm", nil, "fixture-front-secret", 30, frontHandler)
	if err != nil {
		t.Fatal(err)
	}
	front := listenFixture(t, func(connection net.Conn) { service.NewConnection(context.Background(), connection, M.Metadata{}) })
	input := fixtureInput{
		Front: fixtureEndpoint{Server: "127.0.0.1", Port: front.Addr().(*net.TCPAddr).Port, Cipher: "aes-128-gcm", Password: "fixture-front-secret"},
		Exit:  fixtureEndpoint{Server: "127.0.0.1", Port: exit.Addr().(*net.TCPAddr).Port, Username: "fixture-user", Password: "fixture-exit-secret"},
	}
	encoded, _ := json.Marshal(input)
	id, err := startSession(encoded, ownedPipe(t), func() (io.Closer, error) { return &countedTunnel{}, nil }, func(int) bool { return true })
	if err != nil {
		t.Fatal(err)
	}
	defer StopSession(id)
	dialed := make(chan struct{})
	go func() {
		connection, _ := tunnel.Proxies()["CHAIN-EXIT"].DialContext(context.Background(), &C.Metadata{NetWork: C.TCP, DstIP: netip.MustParseAddr("192.0.2.1"), DstPort: 443})
		if connection != nil {
			connection.Close()
		}
		close(dialed)
	}()
	select {
	case <-entered:
	case <-time.After(time.Second):
		t.Fatal("SOCKS handshake did not enter")
	}
	if err := StopSession(id); err != nil {
		t.Fatal(err)
	}
	select {
	case <-dialed:
	case <-time.After(100 * time.Millisecond):
		t.Fatal("stop returned while an old SOCKS handshake was still active")
	}
}

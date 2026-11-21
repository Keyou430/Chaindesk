package engine

import (
	"context"
	"errors"
	"net"
	"net/netip"
	"testing"
	"time"

	"github.com/metacubex/mihomo/component/dialer"
)

func TestSocketProtectionControlsActualConnections(t *testing.T) {
	tests := []struct {
		name    string
		enabled bool
		allowed bool
		stopped bool
		wantErr error
	}{
		{"unconfigured", false, false, false, ErrSocketProtectionUnavailable},
		{"denied", true, false, false, ErrSocketProtectionFailed},
		{"stopped", true, true, true, ErrSocketProtectionUnavailable},
		{"allowed", true, true, false, nil},
	}
	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			listener, err := net.ListenTCP("tcp4", &net.TCPAddr{IP: net.IPv4(127, 0, 0, 1)})
			if err != nil {
				t.Fatal(err)
			}
			defer listener.Close()
			var protector SocketProtector
			if tt.enabled {
				if err := protector.Enable(func(int) bool { return tt.allowed }); err != nil {
					t.Fatal(err)
				}
			}
			if tt.stopped {
				protector.Block()
			}
			ctx, cancel := context.WithTimeout(context.Background(), time.Second)
			defer cancel()
			dialer := net.Dialer{Control: protector.Control}
			connection, err := dialer.DialContext(ctx, "tcp4", listener.Addr().String())
			if connection != nil {
				connection.Close()
			}
			if !errors.Is(err, tt.wantErr) {
				t.Fatalf("dial error = %v, want %v", err, tt.wantErr)
			}
			if err := listener.SetDeadline(time.Now().Add(50 * time.Millisecond)); err != nil {
				t.Fatal(err)
			}
			accepted, acceptErr := listener.Accept()
			if accepted != nil {
				accepted.Close()
			}
			if tt.wantErr == nil && acceptErr != nil {
				t.Fatalf("protected connection did not reach listener: %v", acceptErr)
			}
			if tt.wantErr != nil && acceptErr == nil {
				t.Fatal("connection reached listener without successful protection")
			}
		})
	}
}

func TestMihomoUDPSocketHookBlocksFailedProtection(t *testing.T) {
	for _, allowed := range []bool{false, true} {
		server, err := net.ListenPacket("udp4", "127.0.0.1:0")
		if err != nil {
			t.Fatal(err)
		}
		t.Cleanup(func() { server.Close() })
		calls := 0
		if err := socketProtector.Enable(func(int) bool { calls++; return allowed }); err != nil {
			t.Fatal(err)
		}
		connection, err := dialer.ListenPacket(context.Background(), "udp4", "127.0.0.1:0", netip.MustParseAddrPort(server.LocalAddr().String()))
		if allowed {
			if err != nil {
				t.Fatal(err)
			}
			if _, err := connection.WriteTo([]byte("dns-transport"), server.LocalAddr()); err != nil {
				t.Fatal(err)
			}
			connection.Close()
		} else if !errors.Is(err, ErrSocketProtectionFailed) {
			t.Fatalf("UDP error = %v, want protection failure", err)
		}
		socketProtector.Block()
		if calls != 1 {
			t.Fatalf("protect callback count = %d", calls)
		}
		server.SetReadDeadline(time.Now().Add(50 * time.Millisecond))
		buffer := make([]byte, 32)
		n, _, readErr := server.ReadFrom(buffer)
		if allowed && (readErr != nil || string(buffer[:n]) != "dns-transport") {
			t.Fatal("protected UDP did not reach the server")
		}
		if !allowed && readErr == nil {
			t.Fatal("UDP sent after failed protection")
		}
	}
}

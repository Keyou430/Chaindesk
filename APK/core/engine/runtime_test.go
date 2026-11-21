package engine

import (
	"context"
	"errors"
	"io"
	"net"
	"os"
	"testing"
	"time"

	"github.com/metacubex/mihomo/component/dialer"
	"github.com/metacubex/mihomo/component/resolver"
	"github.com/metacubex/mihomo/tunnel"
)

type countedTunnel struct{ closes int }

func (t *countedTunnel) Close() error { t.closes++; return nil }

func ownedPipe(t *testing.T) *os.File {
	t.Helper()
	reader, writer, err := os.Pipe()
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { reader.Close(); writer.Close() })
	return reader
}

func TestRuntimeRejectsSocketsOutsideFront(t *testing.T) {
	id, err := startSession([]byte(validFixture), ownedPipe(t), func() (io.Closer, error) { return &countedTunnel{}, nil }, func(int) bool { t.Error("unexpected destination reached platform callback"); return true })
	if err != nil {
		t.Fatal(err)
	}
	defer StopSession(id)
	server, err := net.Listen("tcp4", "127.0.0.1:0")
	if err != nil {
		t.Fatal(err)
	}
	defer server.Close()
	ctx, cancel := context.WithTimeout(context.Background(), time.Second)
	defer cancel()
	connection, err := dialer.DialContext(ctx, "tcp4", server.Addr().String())
	if connection != nil {
		connection.Close()
	}
	if !errors.Is(err, ErrSocketDestinationRejected) {
		t.Fatalf("unexpected direct dial result: %v", err)
	}
}

func TestRuntimeStopClearsDNSAndCredentialReferences(t *testing.T) {
	id, err := startSession([]byte(validFixture), ownedPipe(t), func() (io.Closer, error) { return &countedTunnel{}, nil }, func(int) bool { return true })
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { StopSession(id) })
	StopSession(id)
	if resolver.DefaultResolver != nil || tunnel.Proxies()["CHAIN-EXIT"] != nil {
		t.Fatal("stopped runtime retained live DNS or credential-bearing proxies")
	}
}

func requireClosed(t *testing.T, file *os.File) {
	t.Helper()
	if file.Fd() != ^uintptr(0) {
		t.Fatal("owned descriptor remains open")
	}
}

func TestRuntimeClosesRejectedAndFailedDescriptors(t *testing.T) {
	for _, input := range []string{`{}`, validFixture} {
		file := ownedPipe(t)
		_, err := startSession([]byte(input), file, func() (io.Closer, error) {
			return nil, errors.New("tun failure")
		}, func(int) bool { return true })
		if err == nil {
			t.Fatal("failed session reported success")
		}
		requireClosed(t, file)
	}
}

func TestRuntimeSingletonGenerationAndFiftyStops(t *testing.T) {
	var previous uint64
	for i := 0; i < 50; i++ {
		file := ownedPipe(t)
		device := &countedTunnel{}
		id, err := startSession([]byte(validFixture), file, func() (io.Closer, error) { return device, nil }, func(int) bool { return true })
		if err != nil {
			t.Fatal(err)
		}
		t.Cleanup(func() { StopSession(id) })
		if id <= previous {
			t.Fatal("session generation did not advance")
		}
		second := ownedPipe(t)
		if _, err := startSession([]byte(validFixture), second, nil, nil); err == nil {
			t.Fatal("second runtime accepted")
		}
		requireClosed(t, second)
		if result := NativeSelfCheck(); result.ErrorCode != "CORE_SESSION_ACTIVE" {
			t.Fatal("active runtime allowed parser probe")
		}
		StopSession(previous)
		if device.closes != 0 {
			t.Fatal("stale stop closed the current TUN")
		}
		StopSession(id)
		StopSession(id)
		if device.closes != 1 {
			t.Fatal("TUN was not closed exactly once")
		}
		requireClosed(t, file)
		previous = id
	}
}

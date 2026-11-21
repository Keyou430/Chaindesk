package engine

import (
	"context"
	"io"
	"net"
	"testing"
	"time"

	C "github.com/metacubex/mihomo/constant"
	"github.com/metacubex/mihomo/listener/sing"
	"github.com/metacubex/mihomo/listener/sing_tun"
	M "github.com/metacubex/sing/common/metadata"
)

func TestSessionTasksCloseAndDrainBeforeReuse(t *testing.T) {
	tasks := newSessionTasks()
	left, right := net.Pipe()
	defer left.Close()
	defer right.Close()
	if !tasks.begin(left) {
		t.Fatal("new session rejected work")
	}
	stopped := make(chan struct{})
	go func() { tasks.close(); close(stopped) }()
	<-tasks.ctx.Done()
	if tasks.begin(right) {
		t.Fatal("closed session admitted new work")
	}
	select {
	case <-stopped:
		t.Fatal("close returned before an in-flight handler drained")
	case <-time.After(20 * time.Millisecond):
	}
	tasks.end(left)
	select {
	case <-stopped:
	case <-time.After(time.Second):
		t.Fatal("session failed to drain")
	}
}

type pendingTunnel struct {
	C.Tunnel
	entered chan struct{}
	finish  chan struct{}
}

func (p *pendingTunnel) HandleTCPConn(net.Conn, *C.Metadata) { close(p.entered); <-p.finish }

type closeTasks struct{ tasks *sessionTasks }

func (c closeTasks) Close() error { c.tasks.close(); return nil }

func TestRuntimeDrainsInFlightHandlerBeforeNextGeneration(t *testing.T) {
	pending := &pendingTunnel{entered: make(chan struct{}), finish: make(chan struct{})}
	base, err := sing.NewListenerHandler(sing.ListenerConfig{Tunnel: pending, Type: C.TUN})
	if err != nil {
		t.Fatal(err)
	}
	handler := &sessionHandler{ListenerHandler: &sing_tun.ListenerHandler{ListenerHandler: base}, tasks: newSessionTasks()}
	id, err := startSession([]byte(validFixture), ownedPipe(t), func() (io.Closer, error) { return closeTasks{handler.tasks}, nil }, func(int) bool { return true })
	if err != nil {
		t.Fatal(err)
	}
	client, server := net.Pipe()
	defer client.Close()
	defer server.Close()
	handled := make(chan struct{})
	go func() {
		handler.NewConnection(context.Background(), server, M.Metadata{Source: M.ParseSocksaddr("127.0.0.1:5000"), Destination: M.ParseSocksaddr("192.0.2.1:443")})
		close(handled)
	}()
	select {
	case <-pending.entered:
	case <-time.After(time.Second):
		t.Fatal("handler did not enter")
	}
	stopped := make(chan error, 1)
	go func() { stopped <- StopSession(id) }()
	<-handler.tasks.ctx.Done()
	newID := make(chan uint64, 1)
	nextFile := ownedPipe(t)
	go func() {
		next, err := startSession([]byte(validFixture), nextFile, func() (io.Closer, error) { return &countedTunnel{}, nil }, func(int) bool { return true })
		if err != nil {
			newID <- 0
		} else {
			newID <- next
		}
	}()
	select {
	case <-newID:
		t.Fatal("next generation started before the old handler drained")
	case <-time.After(20 * time.Millisecond):
	}
	close(pending.finish)
	<-handled
	if err := <-stopped; err != nil {
		t.Fatal(err)
	}
	next := <-newID
	if next <= id {
		t.Fatal("next generation did not start")
	}
	defer StopSession(next)
	BlockSockets(id)
}

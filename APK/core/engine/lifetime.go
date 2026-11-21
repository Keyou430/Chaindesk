package engine

import (
	"context"
	"net"
	"sync"
)

type sessionTasks struct {
	ctx         context.Context
	cancel      context.CancelFunc
	mu          sync.Mutex
	closed      bool
	connections map[net.Conn]struct{}
	workers     sync.WaitGroup
}

func newSessionTasks() *sessionTasks {
	ctx, cancel := context.WithCancel(context.Background())
	return &sessionTasks{ctx: ctx, cancel: cancel, connections: make(map[net.Conn]struct{})}
}

func (t *sessionTasks) begin(connection net.Conn) bool {
	t.mu.Lock()
	defer t.mu.Unlock()
	if t.closed {
		return false
	}
	if connection != nil {
		t.connections[connection] = struct{}{}
	}
	t.workers.Add(1)
	return true
}

func (t *sessionTasks) end(connection net.Conn) {
	t.mu.Lock()
	delete(t.connections, connection)
	t.mu.Unlock()
	t.workers.Done()
}

func (t *sessionTasks) close() {
	t.mu.Lock()
	t.closed = true
	t.cancel()
	connections := make([]net.Conn, 0, len(t.connections))
	for connection := range t.connections {
		connections = append(connections, connection)
	}
	t.mu.Unlock()
	for _, connection := range connections {
		connection.Close()
	}
	t.workers.Wait()
}

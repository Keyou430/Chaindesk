package engine

import (
	"errors"
	"io"
	"net"
	"testing"
	"time"

	"github.com/metacubex/mihomo/transport/socks5"
)

func TestSOCKSRejectsUnnegotiatedNoAuthentication(t *testing.T) {
	client, server := net.Pipe()
	defer client.Close()
	defer server.Close()
	client.SetDeadline(time.Now().Add(time.Second))
	wroteConnect := make(chan bool, 1)
	go func() {
		defer server.Close()
		greeting := make([]byte, 3)
		io.ReadFull(server, greeting)
		server.Write([]byte{5, 0})
		request := make([]byte, 3)
		n, _ := server.Read(request)
		wroteConnect <- n > 0
	}()
	_, err := socks5.ClientHandshake(client, socks5.ParseAddr("127.0.0.1:443"), socks5.CmdConnect, &socks5.User{Username: "fixture-user", Password: "fixture-password"})
	client.Close()
	if !errors.Is(err, socks5.ErrAuth) {
		t.Fatalf("downgrade error = %v, want ErrAuth", err)
	}
	if <-wroteConnect {
		t.Fatal("CONNECT sent after authentication downgrade")
	}
}

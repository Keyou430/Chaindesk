//go:build android

package engine

import (
	"errors"
	"io"
	"net/netip"
	"os"
	"time"

	"github.com/metacubex/mihomo/config"
	C "github.com/metacubex/mihomo/constant"
	"github.com/metacubex/mihomo/listener/sing"
	"github.com/metacubex/mihomo/listener/sing_tun"
	"github.com/metacubex/mihomo/log"
	"github.com/metacubex/mihomo/tunnel"
	tun "github.com/metacubex/sing-tun"
	"golang.org/x/sys/unix"
)

type androidTunnel struct {
	stack   tun.Stack
	device  tun.Tun
	handler *sessionHandler
}

func (t *androidTunnel) Close() error {
	t.handler.tasks.close()
	// gVisor detaches and joins raw-fd readers before the descriptor can be reused.
	return errors.Join(t.stack.Close(), t.device.Close())
}

// StartSession consumes the copied fd, including on failure.
func StartSession(data []byte, fd int, protect func(int) bool) (uint64, error) {
	if fd < 0 {
		return 0, errors.New("CORE_TUN_FD_INVALID")
	}
	descriptor := os.NewFile(uintptr(fd), "chaindesk-tun-owner")
	return startSession(data, descriptor, func() (io.Closer, error) {
		return newAndroidTunnel(descriptor)
	}, protect)
}

func StartChain(data []byte, fd int, protect func(int) bool) (uint64, error) {
	if fd < 0 {
		return 0, errors.New("CORE_TUN_FD_INVALID")
	}
	descriptor := os.NewFile(uintptr(fd), "chaindesk-chain-tun-owner")
	return startConfiguredSession(data, descriptor, func() (io.Closer, error) { return newAndroidTunnel(descriptor) }, protect, func(data []byte) (*config.Config, string, error) {
		return parseRuntimeChain(data)
	})
}

func newAndroidTunnel(owner *os.File) (_ io.Closer, err error) {
	// sing-tun reserves descriptor zero for creating a new TUN itself.
	fd, err := unix.FcntlInt(owner.Fd(), unix.F_DUPFD_CLOEXEC, 3)
	if err != nil {
		return nil, err
	}
	options := tun.Options{
		Name: "chaindesk", MTU: 1500, FileDescriptor: fd, DNSMode: tun.DNSModeDisabled,
		Inet4Address: []netip.Prefix{netip.MustParsePrefix("172.29.0.1/30")},
	}
	// In pinned sing-tun v0.4.28, the external-fd branch adopts fd without
	// configuration or a subsequent failure path (tun_linux.go:69-76).
	device, err := tun.New(options)
	if err != nil {
		unix.Close(fd)
		return nil, err
	}
	defer func() {
		if err != nil {
			device.Close()
		}
	}()
	handler, err := sing.NewListenerHandler(sing.ListenerConfig{Tunnel: tunnel.Tunnel, Type: C.TUN})
	if err != nil {
		return nil, err
	}
	tasks := newSessionTasks()
	defer func() {
		if err != nil {
			tasks.close()
		}
	}()
	guard := &sessionHandler{
		tasks: tasks,
		ListenerHandler: &sing_tun.ListenerHandler{
			ListenerHandler: handler, Inet4Address: options.Inet4Address,
			DnsAddrPorts:          []netip.AddrPort{netip.MustParseAddrPort("0.0.0.0:53")},
			DisableICMPForwarding: true,
		},
	}
	stack, err := tun.NewStack("gvisor", tun.StackOptions{
		Context: tasks.ctx, Tun: device, TunOptions: options, UDPTimeout: time.Second * 30,
		ICMPTimeout: time.Second, Logger: log.SingLogger, Handler: guard,
	})
	if err != nil {
		return nil, err
	}
	if err = stack.Start(); err != nil {
		stack.Close()
		return nil, err
	}
	return &androidTunnel{stack: stack, device: device, handler: guard}, nil
}

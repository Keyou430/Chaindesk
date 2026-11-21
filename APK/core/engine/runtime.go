package engine

import (
	"errors"
	"io"
	"sync"
	"sync/atomic"

	"github.com/metacubex/mihomo/component/dialer"
	"github.com/metacubex/mihomo/component/resolver"
	"github.com/metacubex/mihomo/config"
	"github.com/metacubex/mihomo/hub/executor"
	"github.com/metacubex/mihomo/tunnel"
	"github.com/metacubex/mihomo/tunnel/statistic"
)

var coreMutex sync.Mutex
var socketProtector SocketProtector
var session struct {
	id       uint64
	next     uint64
	device   io.Closer
	tun      io.Closer
	probes   *sessionTasks
	verified *atomic.Bool
}

func init() { dialer.DefaultSocketHook = socketProtector.Control }

func startSession(data []byte, descriptor io.Closer, startTun func() (io.Closer, error), protect func(int) bool) (uint64, error) {
	return startConfiguredSession(data, descriptor, startTun, protect, func(data []byte) (*config.Config, string, error) {
		cfg, err := parseFixture(data)
		return cfg, "CHAIN-FRONT", err
	})
}

func startConfiguredSession(data []byte, descriptor io.Closer, startTun func() (io.Closer, error), protect func(int) bool, parse func([]byte) (*config.Config, string, error)) (uint64, error) {
	coreMutex.Lock()
	defer coreMutex.Unlock()
	accepted := false
	defer func() {
		if !accepted {
			descriptor.Close()
		}
	}()
	if session.id != 0 {
		return 0, errors.New("CORE_SESSION_ACTIVE")
	}
	cfg, frontName, err := parse(data)
	if err != nil {
		return 0, err
	}
	executor.ApplyConfig(cfg, true)
	session.next++
	if err = socketProtector.enableSession(protect, cfg.Proxies[frontName].Addr(), session.next); err != nil {
		stopRuntime()
		return 0, err
	}
	socketProtector.setLocalPolicy(cfg.Proxies["CHAIN_DESK_EXIT"] != nil)
	socketProtector.setSplitRouting(cfg.Proxies["CHAIN_DESK_EXIT"] != nil && cfg.General.GeodataMode)
	if cfg.Proxies["CHAIN_DESK_EXIT"] != nil {
		tunnel.OnSuspend()
	}
	tun, err := startTun()
	if err != nil {
		stopRuntime()
		return 0, errors.New("CORE_TUN_START_FAILED")
	}
	session.id = session.next
	session.device = descriptor
	session.tun = tun
	session.probes = newSessionTasks()
	session.verified = new(atomic.Bool)
	accepted = true
	return session.id, nil
}

func StopSession(id uint64) error {
	coreMutex.Lock()
	defer coreMutex.Unlock()
	if session.id == 0 || session.id != id {
		return nil
	}
	socketProtector.Block()
	tunnel.OnSuspend()
	session.probes.close()
	tunErr := session.tun.Close()
	err := errors.Join(tunErr, stopRuntime(), session.device.Close())
	session.id = 0
	session.tun = nil
	session.probes = nil
	session.verified = nil
	session.device = nil
	return err
}

func BlockSockets(id uint64) { socketProtector.blockSession(id) }

func ActivateSession(id uint64) bool {
	coreMutex.Lock()
	defer coreMutex.Unlock()
	if id == 0 || session.id != id || session.verified == nil || !session.verified.Load() || session.probes == nil || session.probes.ctx.Err() != nil {
		return false
	}
	tunnel.OnRunning()
	return true
}

func stopRuntime() error {
	socketProtector.Block()
	tunnel.OnSuspend()
	statistic.DefaultManager.Range(func(connection statistic.Tracker) bool { connection.Close(); return true })
	resolver.ResetConnection()
	for _, provider := range tunnel.Providers() {
		if closer, ok := provider.(io.Closer); ok {
			closer.Close()
		}
	}
	for _, proxy := range tunnel.Proxies() {
		proxy.Close()
	}
	cfg, err := executor.ParseWithBytes([]byte(blockedConfiguration))
	if err != nil {
		return errors.New("CORE_STOP_FAILED")
	}
	executor.ApplyConfig(cfg, true)
	tunnel.OnSuspend()
	return nil
}

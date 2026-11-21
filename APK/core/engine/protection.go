package engine

import (
	"errors"
	"net/netip"
	"strings"
	"sync"
	"syscall"
)

var (
	ErrSocketProtectionUnavailable = errors.New("platform socket protection is unavailable")
	ErrSocketProtectionFailed      = errors.New("platform socket protection failed")
	ErrSocketDestinationRejected   = errors.New("outer socket destination is not the fixed front")
)

type SocketProtector struct {
	mu           sync.RWMutex
	protect      func(int) bool
	endpoint     string
	generation   uint64
	allowLocal   bool
	splitRouting bool
}

func (p *SocketProtector) Enable(protect func(int) bool) error {
	if protect == nil {
		return ErrSocketProtectionUnavailable
	}
	p.mu.Lock()
	defer p.mu.Unlock()
	p.protect = protect
	p.endpoint = ""
	p.generation = 0
	p.allowLocal = false
	p.splitRouting = false
	return nil
}

func (p *SocketProtector) enableSession(protect func(int) bool, endpoint string, generation uint64) error {
	if protect == nil {
		return ErrSocketProtectionUnavailable
	}
	p.mu.Lock()
	defer p.mu.Unlock()
	p.protect, p.endpoint, p.generation = protect, endpoint, generation
	p.allowLocal = false
	p.splitRouting = false
	return nil
}

func (p *SocketProtector) setLocalPolicy(allowed bool) {
	p.mu.Lock()
	defer p.mu.Unlock()
	p.allowLocal = allowed
}

func (p *SocketProtector) setSplitRouting(allowed bool) {
	p.mu.Lock()
	defer p.mu.Unlock()
	p.splitRouting = allowed
}

func (p *SocketProtector) blockSession(generation uint64) {
	p.mu.Lock()
	defer p.mu.Unlock()
	if generation != 0 && p.generation == generation {
		p.protect = nil
	}
}

// Block waits for active callbacks before their JNI references may be released.
func (p *SocketProtector) Block() {
	p.mu.Lock()
	defer p.mu.Unlock()
	p.protect = nil
}

func (p *SocketProtector) Control(network, address string, connection syscall.RawConn) error {
	p.mu.RLock()
	defer p.mu.RUnlock()
	if p.protect == nil {
		return ErrSocketProtectionUnavailable
	}
	if p.endpoint != "" {
		allowed := strings.HasPrefix(network, "tcp") && (address == p.endpoint || p.allowLocal && localTCPDestination(address))
		if p.splitRouting {
			endpoint, err := netip.ParseAddrPort(address)
			if err == nil && endpoint.Addr().Is4() && !endpoint.Addr().IsUnspecified() && !endpoint.Addr().IsMulticast() && endpoint.Port() != 0 {
				allowed = strings.HasPrefix(network, "tcp") || strings.HasPrefix(network, "udp") && (address == "223.5.5.5:53" || address == "119.29.29.29:53")
			}
		}
		if !allowed {
			return ErrSocketDestinationRejected
		}
	}
	var protectionErr error
	if err := connection.Control(func(fd uintptr) {
		if !p.protect(int(fd)) {
			protectionErr = ErrSocketProtectionFailed
		}
	}); err != nil {
		return err
	}
	return protectionErr
}

func localTCPDestination(value string) bool {
	endpoint, err := netip.ParseAddrPort(value)
	if err != nil || !endpoint.Addr().Is4() || endpoint.Port() == 0 {
		return false
	}
	for _, prefix := range []string{"127.0.0.0/8", "10.0.0.0/8", "172.16.0.0/12", "192.168.0.0/16", "169.254.0.0/16"} {
		if netip.MustParsePrefix(prefix).Contains(endpoint.Addr()) {
			return true
		}
	}
	return false
}

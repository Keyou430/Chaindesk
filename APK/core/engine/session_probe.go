package engine

import (
	"context"
	"crypto/tls"
	"net"
	"net/http"
	"net/netip"
	"strconv"
	"time"

	C "github.com/metacubex/mihomo/constant"
	"github.com/metacubex/mihomo/tunnel"
)

func CancelProbe(id uint64) {
	coreMutex.Lock()
	defer coreMutex.Unlock()
	if id != 0 && session.id == id && session.probes != nil {
		session.probes.cancel()
	}
}

func ProbeSession(id uint64, path, expected string, budget time.Duration) HealthResult {
	coreMutex.Lock()
	if id == 0 || session.id != id || session.probes == nil {
		coreMutex.Unlock()
		return HealthResult{ErrorCode: "CORE_SESSION_STALE"}
	}
	name := "CHAIN_DESK_EXIT"
	if path == "front" {
		name = "CHAIN_DESK_FRONT"
	} else if path != "exit" {
		coreMutex.Unlock()
		return HealthResult{ErrorCode: "PROBE_PATH_INVALID"}
	}
	proxy := tunnel.Proxies()[name]
	if proxy == nil {
		if path == "front" {
			proxy = tunnel.Proxies()["CHAIN-FRONT"]
		} else {
			proxy = tunnel.Proxies()["CHAIN-EXIT"]
		}
	}
	tasks := session.probes
	verified := session.verified
	if proxy == nil || budget <= 0 || !tasks.begin(nil) {
		coreMutex.Unlock()
		return HealthResult{ErrorCode: "PROBE_UNAVAILABLE"}
	}
	coreMutex.Unlock()
	defer tasks.end(nil)
	ctx, cancel := context.WithTimeout(tasks.ctx, min(budget, 12*time.Second))
	defer cancel()
	transport := &http.Transport{
		TLSClientConfig:   &tls.Config{MinVersion: tls.VersionTLS12},
		DisableKeepAlives: true,
		DialContext: func(ctx context.Context, _, address string) (net.Conn, error) {
			host, port, err := net.SplitHostPort(address)
			if err != nil {
				return nil, err
			}
			value, err := strconv.ParseUint(port, 10, 16)
			if err != nil {
				return nil, err
			}
			metadata := &C.Metadata{NetWork: C.TCP, Type: C.INNER, Host: host, DstPort: uint16(value)}
			if parsed, err := netip.ParseAddr(host); err == nil {
				metadata.DstIP, metadata.Host = parsed, ""
			}
			return proxy.DialContext(ctx, metadata)
		},
	}
	defer transport.CloseIdleConnections()
	client := &http.Client{Transport: transport, CheckRedirect: func(*http.Request, []*http.Request) error { return http.ErrUseLastResponse }}
	result := inspectChainHealth(ctx, client, defaultHealthTargets, expected)
	if path == "exit" && verified != nil && tasks.ctx.Err() == nil {
		verified.Store(result.Healthy)
	}
	return result
}

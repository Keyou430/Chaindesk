package engine

import (
	"context"
	"encoding/json"
	"errors"
	"io"
	"net"
	"path/filepath"
	"testing"
	"time"

	"github.com/Keyou430/Chaindesk/APK/core/chain"
	"github.com/Keyou430/Chaindesk/APK/core/subscription"
	"github.com/metacubex/mihomo/component/geodata"
	C "github.com/metacubex/mihomo/constant"
	"github.com/metacubex/mihomo/tunnel"
)

func TestAllProxyDoesNotInheritChinaDirectSocketPermissions(t *testing.T) {
	originalHome, originalMode := C.Path.HomeDir(), geodata.GeodataMode()
	t.Cleanup(func() { C.SetHomeDir(originalHome); geodata.SetGeodataMode(originalMode) })
	rules, err := filepath.Abs("../../android/app/src/main/assets/rules")
	if err != nil {
		t.Fatal(err)
	}
	if err := SetRulesDirectory(rules); err != nil {
		t.Fatal(err)
	}
	var input runtimeChainInput
	if err := json.Unmarshal(runtimeChainFixture(t), &input); err != nil {
		t.Fatal(err)
	}
	input.Input.Preset, input.Input.RuleVersion = "chinaDirect", chain.RuleVersion
	encoded, _ := json.Marshal(input)
	start := func(data []byte) uint64 {
		t.Helper()
		id, err := startConfiguredSession(data, ownedPipe(t), func() (io.Closer, error) { return &countedTunnel{}, nil }, func(int) bool { return true }, parseRuntimeChain)
		if err != nil {
			t.Fatal(err)
		}
		return id
	}
	if err := StopSession(start(encoded)); err != nil {
		t.Fatal(err)
	}
	id := start(runtimeChainFixture(t))
	defer StopSession(id)
	connection, err := net.ListenUDP("udp4", &net.UDPAddr{IP: net.IPv4(127, 0, 0, 1)})
	if err != nil {
		t.Fatal(err)
	}
	defer connection.Close()
	raw, err := connection.SyscallConn()
	if err != nil {
		t.Fatal(err)
	}
	for _, target := range []struct{ network, address string }{{"tcp4", "198.51.100.41:443"}, {"udp4", "223.5.5.5:53"}} {
		if err := socketProtector.Control(target.network, target.address, raw); !errors.Is(err, ErrSocketDestinationRejected) {
			t.Fatalf("allProxy retained split-routing permission for %s: %v", target.address, err)
		}
	}
}

func runtimeChainFixture(t *testing.T) []byte {
	t.Helper()
	nodes, err := subscription.Parse([]byte("proxies: [{name: 前置, type: ss, server: front.example, port: 443, cipher: aes-128-gcm, password: fixture-secret}]"))
	if err != nil {
		t.Fatal(err)
	}
	encoded, err := json.Marshal(runtimeChainInput{
		Input:     chain.Input{Front: nodes.Nodes[0], Exit: chain.Endpoint{Host: "exit.example", Port: 1080, Username: "u", Password: "p"}, Preset: "allProxy"},
		FrontIPv4: "192.0.2.8", ExitIPv4: "192.0.2.9",
	})
	if err != nil {
		t.Fatal(err)
	}
	return encoded
}

type contextBlockedProxy struct {
	C.Proxy
	entered chan struct{}
}

func (p *contextBlockedProxy) DialContext(ctx context.Context, _ *C.Metadata) (C.Conn, error) {
	close(p.entered)
	<-ctx.Done()
	return nil, ctx.Err()
}

func TestStopCompletesWhileNativeProbeIsBeingCanceled(t *testing.T) {
	id, err := startConfiguredSession(runtimeChainFixture(t), ownedPipe(t), func() (io.Closer, error) { return &countedTunnel{}, nil }, func(int) bool { return true }, parseRuntimeChain)
	if err != nil {
		t.Fatal(err)
	}
	proxy := &contextBlockedProxy{Proxy: tunnel.Proxies()["CHAIN_DESK_EXIT"], entered: make(chan struct{})}
	tunnel.Proxies()["CHAIN_DESK_EXIT"] = proxy
	probeDone := make(chan struct{})
	go func() { ProbeSession(id, "exit", "", time.Second*12); close(probeDone) }()
	select {
	case <-proxy.entered:
	case <-time.After(time.Second):
		t.Fatal("probe did not start")
	}
	stopped := make(chan struct{})
	go func() { StopSession(id); close(stopped) }()
	select {
	case <-stopped:
	case <-time.After(time.Second):
		t.Fatal("stop deadlocked while draining canceled probe")
	}
	select {
	case <-probeDone:
	case <-time.After(time.Second):
		t.Fatal("probe did not return after stop")
	}
}

func TestProductTunStartsSuspendedBeforeItsReaderCanRun(t *testing.T) {
	pausedBeforeReader := false
	id, err := startConfiguredSession(runtimeChainFixture(t), ownedPipe(t), func() (io.Closer, error) {
		pausedBeforeReader = tunnel.Status() == tunnel.Suspend
		return &countedTunnel{}, nil
	}, func(int) bool { return true }, parseRuntimeChain)
	if err != nil {
		t.Fatal(err)
	}
	defer StopSession(id)
	if !pausedBeforeReader {
		t.Fatal("product TUN could forward user traffic before exit verification")
	}
}

func TestRuntimeChainUsesOnlyExplicitBootstrapAddresses(t *testing.T) {
	encoded := runtimeChainFixture(t)
	cfg, frontName, err := parseRuntimeChain(encoded)
	if err != nil {
		t.Fatal(err)
	}
	if cfg.Proxies[frontName].Addr() != "192.0.2.8:443" || cfg.Proxies["CHAIN_DESK_FIXED_EXIT"].Addr() != "192.0.2.9:1080" {
		t.Fatal("runtime did not use the prepared physical-network endpoints")
	}
	var input runtimeChainInput
	json.Unmarshal(encoded, &input)
	input.FrontIPv4 = ""
	invalid, _ := json.Marshal(input)
	if _, _, err := parseRuntimeChain(invalid); err == nil || err.Error() != "BOOTSTRAP_REQUIRED" {
		t.Fatalf("unresolved front accepted: %v", err)
	}
}

func TestProductSessionStopCancelsAndDrainsProbeBeforeReplacement(t *testing.T) {
	id, err := startConfiguredSession(runtimeChainFixture(t), ownedPipe(t), func() (io.Closer, error) { return &countedTunnel{}, nil }, func(int) bool { return true }, parseRuntimeChain)
	if err != nil {
		t.Fatal(err)
	}
	defer StopSession(id)
	if result := ProbeSession(id+1, "exit", "", time.Second); result.ErrorCode != "CORE_SESSION_STALE" {
		t.Fatalf("stale session could probe: %+v", result)
	}
	if !session.probes.begin(nil) {
		t.Fatal("probe ownership was not available")
	}
	ended := make(chan struct{})
	go func() { <-session.probes.ctx.Done(); session.probes.end(nil); close(ended) }()
	if err := StopSession(id); err != nil {
		t.Fatal(err)
	}
	select {
	case <-ended:
	case <-time.After(time.Second):
		t.Fatal("stop returned before draining canceled probe")
	}
	if result := ProbeSession(id, "exit", "", time.Second); result.Healthy || result.ErrorCode != "CORE_SESSION_STALE" {
		t.Fatalf("stopped session could probe: %+v", result)
	}
}

func TestProductSocketPolicyDoesNotPermitPublicDirectOrUDP(t *testing.T) {
	var protector SocketProtector
	if err := protector.enableSession(func(int) bool { return true }, "192.0.2.8:443", 1); err != nil {
		t.Fatal(err)
	}
	protector.allowLocal = true
	for _, destination := range []struct{ network, address string }{{"tcp4", "1.1.1.1:443"}, {"udp4", "10.0.0.1:53"}, {"tcp6", "[::1]:443"}} {
		if err := protector.Control(destination.network, destination.address, nil); !errors.Is(err, ErrSocketDestinationRejected) {
			t.Fatalf("unexpected outer socket allowed: %s %s: %v", destination.network, destination.address, err)
		}
	}
	if result := inspectChainHealth(context.Background(), nil, healthTargets{}, "bad-ip"); result.ErrorCode != "EXPECTED_IP_INVALID" {
		t.Fatal(result)
	}
}

func TestCancelProbeRejectsStaleGeneration(t *testing.T) {
	id, err := startConfiguredSession(runtimeChainFixture(t), ownedPipe(t), func() (io.Closer, error) { return &countedTunnel{}, nil }, func(int) bool { return true }, parseRuntimeChain)
	if err != nil {
		t.Fatal(err)
	}
	defer StopSession(id)
	CancelProbe(id + 1)
	select {
	case <-session.probes.ctx.Done():
		t.Fatal("stale cancellation reached current probe")
	default:
	}
	CancelProbe(id)
	select {
	case <-session.probes.ctx.Done():
	case <-time.After(time.Second):
		t.Fatal("current probe was not canceled")
	}
}

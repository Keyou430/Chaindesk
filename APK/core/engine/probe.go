package engine

import (
	"runtime"

	"github.com/metacubex/mihomo/constant"
	"github.com/metacubex/mihomo/hub/executor"
)

type ProbeResult struct {
	CoreVersion     string `json:"coreVersion"`
	GoVersion       string `json:"goVersion"`
	Target          string `json:"target"`
	Goroutines      int    `json:"goroutines"`
	ConfigValidated bool   `json:"configValidated"`
	VPNStarted      bool   `json:"vpnStarted"`
	ErrorCode       string `json:"errorCode,omitempty"`
}

const blockedConfiguration = `
mode: rule
log-level: silent
ipv6: false
find-process-mode: off
geo-auto-update: false
geodata-mode: false
dns: {enable: false}
tun: {enable: false}
profile: {store-selected: false, store-fake-ip: false}
proxies: []
proxy-groups: []
rules: ["MATCH,REJECT"]
`

func NativeSelfCheck() ProbeResult {
	// config.Parse temporarily changes mihomo package globals.
	coreMutex.Lock()
	defer coreMutex.Unlock()
	result := ProbeResult{
		CoreVersion: constant.Version,
		GoVersion:   runtime.Version(),
		Target:      runtime.GOOS + "/" + runtime.GOARCH,
		Goroutines:  runtime.NumGoroutine(),
	}
	if session.id != 0 {
		result.ErrorCode = "CORE_SESSION_ACTIVE"
		return result
	}
	parsed, err := executor.ParseWithBytes([]byte(blockedConfiguration))
	if err == nil && !parsed.General.Tun.Enable && parsed.General.MixedPort == 0 && parsed.General.Port == 0 && parsed.General.SocksPort == 0 {
		result.ConfigValidated = true
	} else {
		result.ErrorCode = "CORE_CONFIG_INVALID"
	}
	return result
}

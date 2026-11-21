package chain

import (
	"encoding/json"
	"errors"
	"unicode/utf8"

	"github.com/Keyou430/Chaindesk/APK/core/subscription"
)

type Endpoint struct {
	Host     string `json:"host"`
	Port     int    `json:"port"`
	Username string `json:"username"`
	Password string `json:"password"`
}

type Input struct {
	Front       subscription.Node `json:"front"`
	Exit        Endpoint          `json:"exit"`
	Preset      string            `json:"preset"`
	RuleVersion string            `json:"ruleVersion,omitempty"`
}

type Result struct {
	Config            string `json:"config"`
	FrontName         string `json:"frontName"`
	CapabilityVersion int    `json:"capabilityVersion"`
}

func Compile(input Input) (Result, error) {
	front, err := subscription.Validate(input.Front)
	if err != nil {
		return Result{}, err
	}
	host, err := subscription.NormalizeHost(input.Exit.Host)
	if err != nil || input.Exit.Port < 1 || input.Exit.Port > 65535 {
		return Result{}, errors.New("EXIT_ADDRESS_INVALID")
	}
	username, password := input.Exit.Username, input.Exit.Password
	if !utf8.ValidString(username) || !utf8.ValidString(password) ||
		(username != "" || password != "") && (len(username) < 1 || len(username) > 255 || len(password) < 1 || len(password) > 255) {
		return Result{}, errors.New("EXIT_AUTH_INVALID")
	}
	if front.Server == host && front.Port == input.Exit.Port {
		return Result{}, errors.New("NO_VALID_FRONT")
	}
	switch input.Preset {
	case "allProxy":
	case "chinaDirect":
		if input.RuleVersion != RuleVersion {
			return Result{}, errors.New("RULE_RESOURCE_UNAVAILABLE")
		}
	default:
		return Result{}, errors.New("RULE_PRESET_INVALID")
	}
	frontName := "CHAIN_DESK_NODE_" + front.ID
	front.Options["name"], front.Options["udp"] = frontName, false
	exit := map[string]any{
		"name": "CHAIN_DESK_FIXED_EXIT", "type": "socks5", "server": host,
		"port": input.Exit.Port, "udp": false, "dialer-proxy": "CHAIN_DESK_FRONT",
	}
	if username != "" {
		exit["username"], exit["password"] = username, password
	}
	configuration := map[string]any{
		"mode": "rule", "ipv6": false, "allow-lan": false, "bind-address": "127.0.0.1",
		"log-level": "silent", "geo-auto-update": false, "geodata-mode": input.Preset == "chinaDirect", "find-process-mode": "off",
		"profile": map[string]any{"store-selected": false, "store-fake-ip": false},
		"tun":     map[string]any{"enable": false},
		"dns": map[string]any{
			"enable": true, "enhanced-mode": "redir-host", "ipv6": false, "prefer-h3": false,
			"default-nameserver":      []string{"223.5.5.5", "119.29.29.29"},
			"proxy-server-nameserver": []string{"223.5.5.5", "119.29.29.29"},
			"nameserver":              []string{"https://1.1.1.1/dns-query#CHAIN_DESK_EXIT"},
		},
		"proxies": []map[string]any{front.Options, exit},
		"proxy-groups": []map[string]any{
			{"name": "CHAIN_DESK_FRONT", "type": "select", "proxies": []string{frontName}},
			{"name": "CHAIN_DESK_EXIT", "type": "select", "proxies": []string{"CHAIN_DESK_FIXED_EXIT"}},
		},
		"rules": []string{
			"NETWORK,udp,REJECT",
			"DOMAIN,www.gstatic.com,CHAIN_DESK_EXIT",
			"DOMAIN,cp.cloudflare.com,CHAIN_DESK_EXIT",
			"DOMAIN,api.ipify.org,CHAIN_DESK_EXIT",
			"DOMAIN,www.cloudflare.com,CHAIN_DESK_EXIT",
			"DOMAIN,localhost,DIRECT", "DOMAIN-SUFFIX,local,DIRECT",
			"IP-CIDR,127.0.0.0/8,DIRECT,no-resolve", "IP-CIDR,10.0.0.0/8,DIRECT,no-resolve",
			"IP-CIDR,172.16.0.0/12,DIRECT,no-resolve", "IP-CIDR,192.168.0.0/16,DIRECT,no-resolve",
			"IP-CIDR,169.254.0.0/16,DIRECT,no-resolve", "MATCH,CHAIN_DESK_EXIT",
		},
	}
	if input.Preset == "chinaDirect" {
		rules := configuration["rules"].([]string)
		configuration["rules"] = append(rules[:len(rules)-1], "GEOSITE,cn,DIRECT", "GEOIP,CN,DIRECT,no-resolve", "MATCH,CHAIN_DESK_EXIT")
		dns := configuration["dns"].(map[string]any)
		dns["direct-nameserver"] = []string{"https://dns.alidns.com/dns-query#DIRECT"}
		dns["nameserver-policy"] = map[string]any{"geosite:cn": []string{"https://dns.alidns.com/dns-query#DIRECT"}}
	}
	encoded, err := json.Marshal(configuration)
	if err != nil {
		return Result{}, errors.New("CHAIN_COMPILE_FAILED")
	}
	return Result{Config: string(encoded), FrontName: frontName, CapabilityVersion: subscription.CapabilityVersion}, nil
}

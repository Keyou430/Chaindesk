package subscription

import (
	"errors"
	"math"
	"strings"
	"unicode/utf8"
)

var optionFields = []string{
	"type", "server", "port", "cipher", "password", "uuid", "alterId", "tls",
	"servername", "sni", "client-fingerprint", "alpn", "flow", "network",
}

// Validate rechecks untrusted DTOs and returns a new, normalized option map.
func Validate(node Node) (Node, error) {
	invalid := errors.New("NO_VALID_FRONT")
	if node.Rejection != "" || !utf8.ValidString(node.Name) || len(node.Options) > len(optionFields) {
		return Node{}, invalid
	}
	fields := make(map[string]any, len(node.Options)+1)
	for key, value := range node.Options {
		known := false
		for _, allowed := range optionFields {
			known = known || key == allowed
		}
		if !known || !optionValue(key, value) {
			return Node{}, invalid
		}
		fields[key] = cloneOption(value)
		if key == "port" || key == "alterId" {
			if number, ok := value.(float64); ok {
				if number < 0 || number > 65535 || math.Trunc(number) != number {
					return Node{}, invalid
				}
				fields[key] = int(number)
			}
		}
	}
	fields["name"] = node.Name
	parsed := parseNode(fields)
	if parsed.Rejection != "" || node.ID != parsed.ID || node.Protocol != parsed.Protocol || node.Server != parsed.Server || node.Port != parsed.Port {
		return Node{}, invalid
	}
	return parsed, nil
}

func integer(value any) (int, bool) {
	switch number := value.(type) {
	case int:
		return number, true
	case int64:
		if number >= 0 && number <= math.MaxInt {
			return int(number), true
		}
	case uint64:
		if number <= math.MaxInt {
			return int(number), true
		}
	}
	return 0, false
}

func optionValue(key string, value any) bool {
	if key == "alpn" {
		_, generic := value.([]any)
		_, strings := value.([]string)
		if !generic && !strings {
			return false
		}
	}
	switch value := value.(type) {
	case string:
		return len(value) <= 4096 && utf8.ValidString(value)
	case bool, int, int64, uint64:
		return true
	case float64:
		return !math.IsNaN(value) && !math.IsInf(value, 0)
	case []string:
		if key != "alpn" || len(value) > 8 {
			return false
		}
		for _, item := range value {
			if len(item) > 32 || !utf8.ValidString(item) {
				return false
			}
		}
		return true
	case []any:
		if key != "alpn" || len(value) > 8 {
			return false
		}
		for _, item := range value {
			text, ok := item.(string)
			if !ok || len(text) > 32 || !utf8.ValidString(text) {
				return false
			}
		}
		return true
	}
	return false
}

func cloneOption(value any) any {
	switch value := value.(type) {
	case []string:
		return append([]string{}, value...)
	case []any:
		return append([]any{}, value...)
	default:
		return value
	}
}

func rejection(fields map[string]any, protocol, server string, port int) string {
	if server == "" || port == 0 {
		return "FRONT_ENDPOINT_INVALID"
	}
	if _, exists := fields["dialer-proxy"]; exists {
		return "FRONT_CHAIN_UNSUPPORTED"
	}
	if value, exists := fields["skip-cert-verify"]; exists {
		insecure, ok := value.(bool)
		if !ok {
			return "FRONT_FIELD_INVALID"
		}
		if insecure {
			return "FRONT_TLS_INSECURE"
		}
	}
	for _, key := range []string{"cipher", "password", "uuid", "sni", "servername", "client-fingerprint", "network", "flow"} {
		if value, exists := fields[key]; exists {
			text, ok := value.(string)
			if !ok || len(text) > 4096 || !utf8.ValidString(text) {
				return "FRONT_FIELD_INVALID"
			}
		}
	}
	if value, exists := fields["tls"]; exists {
		if _, ok := value.(bool); !ok {
			return "FRONT_FIELD_INVALID"
		}
	}
	if value, exists := fields["alpn"]; exists {
		if !optionValue("alpn", value) {
			return "FRONT_FIELD_INVALID"
		}
	}
	if network, exists := fields["network"]; exists && network != "tcp" {
		return "FRONT_TRANSPORT_UNSUPPORTED"
	}
	for _, key := range []string{"plugin", "reality-opts"} {
		if _, exists := fields[key]; exists {
			return "FRONT_TRANSPORT_UNSUPPORTED"
		}
	}
	password, _ := fields["password"].(string)
	uuid, _ := fields["uuid"].(string)
	cipher, _ := fields["cipher"].(string)
	switch protocol {
	case "ss":
		if cipher != "aes-128-gcm" && cipher != "aes-256-gcm" && cipher != "chacha20-ietf-poly1305" {
			return "FRONT_CIPHER_UNSUPPORTED"
		}
		if password == "" {
			return "FRONT_AUTH_INVALID"
		}
	case "vmess", "vless":
		if protocol == "vless" && fields["tls"] != true {
			return "FRONT_TLS_INSECURE"
		}
		if protocol == "vmess" {
			if _, exists := fields["cipher"]; exists && cipher != "auto" {
				return "FRONT_CIPHER_UNSUPPORTED"
			}
			if value, exists := fields["alterId"]; exists {
				if number, ok := integer(value); !ok || number != 0 {
					return "FRONT_TRANSPORT_UNSUPPORTED"
				}
			}
		}
		if !validUUID(uuid) {
			return "FRONT_AUTH_INVALID"
		}
		if flow, _ := fields["flow"].(string); flow != "" {
			return "FRONT_TRANSPORT_UNSUPPORTED"
		}
	case "trojan", "anytls":
		if password == "" {
			return "FRONT_AUTH_INVALID"
		}
	default:
		return "FRONT_PROTOCOL_UNSUPPORTED"
	}
	return ""
}

func validUUID(value string) bool {
	if len(value) != 36 {
		return false
	}
	for index, char := range []byte(value) {
		if index == 8 || index == 13 || index == 18 || index == 23 {
			if char != '-' {
				return false
			}
		} else if !strings.ContainsRune("0123456789abcdefABCDEF", rune(char)) {
			return false
		}
	}
	return true
}

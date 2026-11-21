package subscription

import (
	"errors"
	"net/netip"
	"net/url"
	"strconv"
	"strings"
	"unicode"
	"unicode/utf8"

	"golang.org/x/net/idna"
)

func NormalizeHost(input string) (string, error) {
	invalid := errors.New("EXIT_ADDRESS_INVALID")
	host := strings.TrimRight(strings.TrimSpace(input), ".")
	if host == "" || len(host) > 253 || !utf8.ValidString(host) {
		return "", invalid
	}
	for _, char := range host {
		if unicode.IsControl(char) || unicode.IsSpace(char) || strings.ContainsRune(":/@?#[]\\", char) {
			return "", invalid
		}
	}
	if address, err := netip.ParseAddr(host); err == nil && address.Is4() {
		return address.String(), nil
	}
	if strings.Trim(host, "0123456789.") == "" {
		return "", invalid
	}
	host, err := url.PathUnescape(host)
	if err != nil {
		return "", invalid
	}
	normalized, err := idna.Lookup.ToASCII(host)
	if err != nil || len(normalized) > 253 {
		return "", invalid
	}
	normalized = strings.ToLower(normalized)
	labels := strings.Split(normalized, ".")
	last := labels[len(labels)-1]
	hexEnding := strings.HasPrefix(last, "0x") && strings.Trim(last[2:], "0123456789abcdef") == ""
	if _, number := ipv4Number(last); number || hexEnding || last != "" && strings.Trim(last, "0123456789") == "" {
		if len(labels) > 4 {
			return "", invalid
		}
		var address uint64
		for index, label := range labels {
			number, valid := ipv4Number(label)
			if !valid {
				return "", invalid
			}
			if index == len(labels)-1 {
				if number >= uint64(1)<<(8*(5-len(labels))) {
					return "", invalid
				}
				address += number
			} else {
				if number > 255 {
					return "", invalid
				}
				address += number << (8 * (3 - index))
			}
		}
		return netip.AddrFrom4([4]byte{byte(address >> 24), byte(address >> 16), byte(address >> 8), byte(address)}).String(), nil
	}
	for _, label := range labels {
		if label == "" || len(label) > 63 || label[0] == '-' || label[len(label)-1] == '-' {
			return "", invalid
		}
		for _, char := range label {
			if !(char >= 'a' && char <= 'z' || char >= '0' && char <= '9' || char == '-') {
				return "", invalid
			}
		}
	}
	return normalized, nil
}

// The desktop URL parser interprets numeric host endings using WHATWG IPv4 rules.
func ipv4Number(input string) (uint64, bool) {
	if input == "" {
		return 0, false
	}
	base := 10
	if strings.HasPrefix(input, "0x") {
		input, base = input[2:], 16
	} else if len(input) > 1 && input[0] == '0' {
		input, base = input[1:], 8
	}
	if input == "" {
		return 0, true
	}
	number, err := strconv.ParseUint(input, base, 64)
	return number, err == nil
}

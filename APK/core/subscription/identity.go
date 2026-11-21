package subscription

import (
	"crypto/sha256"
	"encoding/hex"
	"encoding/json"
	"sort"
)

func stableID(options map[string]any) string {
	keys := make([]string, 0, len(options))
	for key := range options {
		keys = append(keys, key)
	}
	sort.Strings(keys)
	canonical := []byte("[1,[")
	for i, key := range keys {
		if i > 0 {
			canonical = append(canonical, ',')
		}
		canonical = append(canonical, '[')
		canonical = appendJSONString(canonical, key)
		canonical = append(canonical, ',')
		canonical = appendJSONValue(canonical, options[key])
		canonical = append(canonical, ']')
	}
	canonical = append(canonical, ']', ']')
	digest := sha256.Sum256(canonical)
	return hex.EncodeToString(digest[:])
}

func appendJSONValue(output []byte, value any) []byte {
	switch value := value.(type) {
	case string:
		return appendJSONString(output, value)
	case []any:
		output = append(output, '[')
		for i, item := range value {
			if i > 0 {
				output = append(output, ',')
			}
			output = appendJSONValue(output, item)
		}
		return append(output, ']')
	case []string:
		output = append(output, '[')
		for i, item := range value {
			if i > 0 {
				output = append(output, ',')
			}
			output = appendJSONString(output, item)
		}
		return append(output, ']')
	default:
		encoded, _ := json.Marshal(value)
		return append(output, encoded...)
	}
}

// serde_json leaves HTML and U+2028/U+2029 unescaped; Go's encoder does not.
func appendJSONString(output []byte, value string) []byte {
	output = append(output, '"')
	for _, char := range value {
		switch char {
		case '"', '\\':
			output = append(output, '\\', byte(char))
		case '\b':
			output = append(output, '\\', 'b')
		case '\f':
			output = append(output, '\\', 'f')
		case '\n':
			output = append(output, '\\', 'n')
		case '\r':
			output = append(output, '\\', 'r')
		case '\t':
			output = append(output, '\\', 't')
		default:
			if char < 0x20 {
				output = append(output, '\\', 'u', '0', '0')
				output = append(output, "0123456789abcdef"[byte(char)>>4], "0123456789abcdef"[byte(char)&15])
			} else {
				output = append(output, string(char)...)
			}
		}
	}
	return append(output, '"')
}

package subscription

import (
	"bytes"
	"errors"
	"io"
	"strings"
	"unicode/utf8"

	"go.yaml.in/yaml/v3"
)

func Parse(source []byte) (Result, error) {
	if len(source) > 10*1024*1024 {
		return Result{}, errors.New("SUBSCRIPTION_TOO_LARGE")
	}
	invalid := errors.New("SUBSCRIPTION_UNSUPPORTED")
	if !utf8.Valid(source) {
		return Result{}, invalid
	}
	for _, line := range bytes.Split(source, []byte{'\n'}) {
		if bytes.HasPrefix(line, []byte("%TAG ")) || bytes.HasPrefix(line, []byte("%TAG\t")) {
			return Result{}, invalid
		}
	}
	decoder := yaml.NewDecoder(bytes.NewReader(source))
	var document yaml.Node
	if decoder.Decode(&document) != nil || len(document.Content) != 1 {
		return Result{}, invalid
	}
	if decoder.Decode(new(yaml.Node)) != io.EOF {
		return Result{}, invalid
	}
	root := document.Content[0]
	if root.Kind != yaml.MappingNode {
		return Result{}, invalid
	}
	if err := inspectTree(root); err != nil {
		return Result{}, err
	}
	var proxies *yaml.Node
	for i := 0; i < len(root.Content); i += 2 {
		if root.Content[i].Value == "proxies" {
			proxies = root.Content[i+1]
		}
	}
	if proxies == nil || proxies.Kind != yaml.SequenceNode {
		return Result{}, invalid
	}
	if len(proxies.Content) > 5000 {
		return Result{}, errors.New("SUBSCRIPTION_TOO_LARGE")
	}
	result := Result{CapabilityVersion: CapabilityVersion, Nodes: []Node{}}
	identities := make(map[string]bool, len(proxies.Content))
	for _, proxy := range proxies.Content {
		if proxy.Kind != yaml.MappingNode {
			return Result{}, invalid
		}
		fields := make(map[string]any, len(proxy.Content)/2)
		for i := 0; i < len(proxy.Content); i += 2 {
			var value any
			if proxy.Content[i+1].Decode(&value) != nil {
				return Result{}, invalid
			}
			fields[proxy.Content[i].Value] = value
		}
		node := parseNode(fields)
		if identities[node.ID] {
			result.Duplicates++
		} else {
			identities[node.ID] = true
			result.Nodes = append(result.Nodes, node)
		}
	}
	return result, nil
}

func inspectTree(root *yaml.Node) error {
	type entry struct {
		node  *yaml.Node
		depth int
	}
	pending := []entry{{root, 0}}
	scalars, total := 0, 0
	for len(pending) > 0 {
		current := pending[len(pending)-1]
		pending = pending[:len(pending)-1]
		node, depth := current.node, current.depth
		total++
		if node.Anchor != "" || node.Kind == yaml.AliasNode || node.Style&yaml.TaggedStyle != 0 {
			return errors.New("SUBSCRIPTION_UNSUPPORTED")
		}
		if node.Kind == yaml.ScalarNode {
			scalars++
		} else {
			depth++
			if depth > 32 {
				return errors.New("SUBSCRIPTION_UNSUPPORTED")
			}
		}
		if scalars > 250000 || total > 1000000 {
			return errors.New("SUBSCRIPTION_TOO_LARGE")
		}
		if node.Kind == yaml.MappingNode {
			keys := make(map[string]bool, len(node.Content)/2)
			for i := 0; i < len(node.Content); i += 2 {
				key := node.Content[i]
				if key.Kind != yaml.ScalarNode || key.Tag != "!!str" || keys[key.Value] {
					return errors.New("SUBSCRIPTION_UNSUPPORTED")
				}
				keys[key.Value] = true
			}
		}
		for _, child := range node.Content {
			pending = append(pending, entry{child, depth})
		}
	}
	return nil
}

func parseNode(fields map[string]any) Node {
	name, ok := fields["name"].(string)
	if !ok {
		name = "Unnamed node"
	}
	nameRunes := []rune(name)
	if len(nameRunes) > 256 {
		name = string(nameRunes[:256])
	}
	protocol, ok := fields["type"].(string)
	if !ok || len(protocol) > 32 {
		protocol = "unsupported"
	}
	protocol = strings.ToLower(protocol)
	server, _ := fields["server"].(string)
	server, _ = NormalizeHost(server)
	port, _ := integer(fields["port"])
	if port < 1 || port > 65535 {
		port = 0
	}
	options := make(map[string]any)
	for _, key := range optionFields {
		value, exists := fields[key]
		if exists && optionValue(key, value) {
			options[key] = cloneOption(value)
		}
	}
	options["type"], options["server"], options["port"] = protocol, server, port
	return Node{
		ID: stableID(options), Name: name, Protocol: protocol, Server: server, Port: port,
		Options: options, Rejection: rejection(fields, protocol, server, port),
	}
}

use super::{CAPABILITY_VERSION, FrontNode, MAX_NODES, MAX_SUBSCRIPTION_BYTES, Subscription, failure, normalize_host};
use anyhow::Result;
use serde_yaml_ng::{Mapping, Value};
use sha2::{Digest as _, Sha256};
use std::{collections::HashSet, mem::MaybeUninit};
use unsafe_libyaml::{
    YAML_ALIAS_TOKEN, YAML_ANCHOR_TOKEN, YAML_BLOCK_END_TOKEN, YAML_BLOCK_MAPPING_START_TOKEN,
    YAML_BLOCK_SEQUENCE_START_TOKEN, YAML_FLOW_MAPPING_END_TOKEN, YAML_FLOW_MAPPING_START_TOKEN,
    YAML_FLOW_SEQUENCE_END_TOKEN, YAML_FLOW_SEQUENCE_START_TOKEN, YAML_SCALAR_TOKEN, YAML_STREAM_END_TOKEN,
    YAML_TAG_DIRECTIVE_TOKEN, YAML_TAG_TOKEN, yaml_parser_delete, yaml_parser_initialize, yaml_parser_scan,
    yaml_parser_set_input_string, yaml_parser_t, yaml_token_delete, yaml_token_t,
};

fn inspect_yaml(input: &str) -> Result<()> {
    if input.len() > MAX_SUBSCRIPTION_BYTES {
        return Err(failure(
            "SUBSCRIPTION_TOO_LARGE",
            "Subscriptions are limited to 10 MiB after decoding.",
        ));
    }
    // Scan tokens before serde can expand aliases or descend into untrusted collections.
    unsafe {
        let mut parser = MaybeUninit::<yaml_parser_t>::uninit();
        if yaml_parser_initialize(parser.as_mut_ptr()).fail {
            return Err(failure(
                "SUBSCRIPTION_UNSUPPORTED",
                "Could not initialize the YAML parser.",
            ));
        }
        let mut parser = scopeguard::guard(parser.assume_init(), |mut parser| yaml_parser_delete(&mut parser));
        yaml_parser_set_input_string(&mut *parser, input.as_ptr(), input.len() as u64);
        let mut depth = 0_usize;
        let mut tokens = 0_usize;
        let mut scalars = 0_usize;
        loop {
            let mut token = MaybeUninit::<yaml_token_t>::uninit();
            if yaml_parser_scan(&mut *parser, token.as_mut_ptr()).fail {
                return Err(failure(
                    "SUBSCRIPTION_UNSUPPORTED",
                    "The subscription is not valid YAML.",
                ));
            }
            let mut token = token.assume_init();
            let kind = token.type_;
            yaml_token_delete(&mut token);
            tokens += 1;
            if kind == YAML_SCALAR_TOKEN {
                scalars += 1;
            }
            if tokens > 1_000_000 || scalars > 250_000 {
                return Err(failure(
                    "SUBSCRIPTION_TOO_LARGE",
                    "The subscription structure exceeds the YAML parsing budget.",
                ));
            }
            match kind {
                YAML_ALIAS_TOKEN | YAML_ANCHOR_TOKEN | YAML_TAG_TOKEN | YAML_TAG_DIRECTIVE_TOKEN => {
                    return Err(failure(
                        "SUBSCRIPTION_UNSUPPORTED",
                        "YAML aliases, anchors and tags are not supported.",
                    ));
                }
                YAML_BLOCK_SEQUENCE_START_TOKEN
                | YAML_BLOCK_MAPPING_START_TOKEN
                | YAML_FLOW_SEQUENCE_START_TOKEN
                | YAML_FLOW_MAPPING_START_TOKEN => {
                    depth += 1;
                    if depth > 32 {
                        return Err(failure(
                            "SUBSCRIPTION_UNSUPPORTED",
                            "YAML nesting is limited to 32 levels.",
                        ));
                    }
                }
                YAML_BLOCK_END_TOKEN | YAML_FLOW_SEQUENCE_END_TOKEN | YAML_FLOW_MAPPING_END_TOKEN => {
                    depth = depth.saturating_sub(1)
                }
                YAML_STREAM_END_TOKEN => return Ok(()),
                _ => {}
            }
        }
    }
}

pub(crate) fn parse(source: String, name: String, update_url: Option<String>) -> Result<Subscription> {
    inspect_yaml(&source)?;
    let root: Mapping = serde_yaml_ng::from_str(&source).map_err(|_| {
        failure(
            "SUBSCRIPTION_UNSUPPORTED",
            "Expected one YAML document with an inline proxies list.",
        )
    })?;
    let proxies = root.get("proxies").and_then(Value::as_sequence).ok_or_else(|| {
        failure(
            "SUBSCRIPTION_UNSUPPORTED",
            "Only inline proxies are supported; import a subscription containing a proxies list.",
        )
    })?;
    let mut pending = root.values().map(|value| (value, 2_usize)).collect::<Vec<_>>();
    while let Some((value, depth)) = pending.pop() {
        match value {
            Value::Mapping(map) => {
                if depth > 32 {
                    return Err(failure(
                        "SUBSCRIPTION_UNSUPPORTED",
                        "YAML nesting is limited to 32 levels.",
                    ));
                }
                for (key, value) in map {
                    pending.extend([(key, depth + 1), (value, depth + 1)]);
                }
            }
            Value::Sequence(sequence) => {
                if depth > 32 {
                    return Err(failure(
                        "SUBSCRIPTION_UNSUPPORTED",
                        "YAML nesting is limited to 32 levels.",
                    ));
                }
                pending.extend(sequence.iter().map(|value| (value, depth + 1)));
            }
            _ => {}
        }
    }
    if proxies.len() > MAX_NODES {
        return Err(failure(
            "SUBSCRIPTION_TOO_LARGE",
            "Subscriptions are limited to 5000 nodes.",
        ));
    }
    let mut nodes = Vec::new();
    let mut identities = HashSet::new();
    let mut duplicates = 0;
    for proxy in proxies {
        let node = parse_node(proxy)?;
        if identities.insert(node.id.clone()) {
            nodes.push(node);
        } else {
            duplicates += 1;
        }
    }
    Ok(Subscription {
        name: name.chars().take(128).collect(),
        source,
        update_url,
        nodes,
        duplicates,
        imported_at: chrono::Utc::now().to_rfc3339(),
    })
}

fn string<'a>(map: &'a Mapping, key: &str) -> Option<&'a str> {
    map.get(key).and_then(Value::as_str)
}

fn parse_node(value: &Value) -> Result<FrontNode> {
    let map = value
        .as_mapping()
        .ok_or_else(|| failure("SUBSCRIPTION_UNSUPPORTED", "Each proxy must be an object."))?;
    let name = string(map, "name")
        .unwrap_or("Unnamed node")
        .chars()
        .take(256)
        .collect::<String>();
    let protocol = string(map, "type")
        .filter(|value| value.len() <= 32)
        .unwrap_or("unsupported")
        .to_ascii_lowercase();
    let server = string(map, "server")
        .and_then(|s| normalize_host(s).ok())
        .unwrap_or_default();
    let port = map
        .get("port")
        .and_then(Value::as_u64)
        .and_then(|n| u16::try_from(n).ok())
        .unwrap_or(0);
    let mut options = Mapping::new();
    for field in [
        "type",
        "server",
        "port",
        "cipher",
        "password",
        "uuid",
        "alterId",
        "tls",
        "servername",
        "sni",
        "client-fingerprint",
        "alpn",
        "flow",
        "network",
    ] {
        if let Some(value) = map.get(field).filter(|value| match value {
            Value::String(value) => value.len() <= 4096,
            Value::Bool(_) | Value::Number(_) => true,
            Value::Sequence(values) if field == "alpn" => {
                values.len() <= 8
                    && values
                        .iter()
                        .all(|value| value.as_str().is_some_and(|value| value.len() <= 32))
            }
            _ => false,
        }) {
            options.insert(field.into(), value.clone());
        }
    }
    options.insert("type".into(), protocol.clone().into());
    options.insert("server".into(), server.clone().into());
    options.insert("port".into(), port.into());
    let rejection = rejection(map, &protocol, &server, port);
    let mut canonical = options
        .iter()
        .map(|(key, value)| (key.as_str().unwrap_or_default(), value))
        .collect::<Vec<_>>();
    canonical.sort_by_key(|(key, _)| *key);
    let bytes = serde_json::to_vec(&(CAPABILITY_VERSION, canonical))?;
    let id = Sha256::digest(bytes).iter().map(|byte| format!("{byte:02x}")).collect();
    Ok(FrontNode {
        id,
        name,
        protocol,
        server,
        port,
        options,
        rejection,
    })
}

fn rejection(map: &Mapping, protocol: &str, server: &str, port: u16) -> Option<String> {
    let reject = |message: &str| Some(message.to_owned());
    if server.is_empty() || port == 0 {
        return reject("Missing or invalid server and port.");
    }
    if map.contains_key("dialer-proxy") {
        return reject("Subscription-defined proxy chains are not supported.");
    }
    if map.get("skip-cert-verify").and_then(Value::as_bool) == Some(true) {
        return reject("Certificate verification must remain enabled.");
    }
    if (map.contains_key("tls") && map.get("tls").and_then(Value::as_bool).is_none())
        || (map.contains_key("alpn")
            && map.get("alpn").and_then(Value::as_sequence).is_none_or(|values| {
                values.len() > 8
                    || values
                        .iter()
                        .any(|value| value.as_str().is_none_or(|value| value.len() > 32))
            }))
    {
        return reject("A TLS option has an invalid type or length.");
    }
    if (map.contains_key("network") && string(map, "network") != Some("tcp"))
        || map.contains_key("plugin")
        || map.contains_key("reality-opts")
    {
        return reject("This transport variant has not passed the chain capability matrix.");
    }
    match protocol {
        "ss" => {
            if !matches!(
                string(map, "cipher"),
                Some("aes-128-gcm" | "aes-256-gcm" | "chacha20-ietf-poly1305")
            ) {
                return reject("This Shadowsocks cipher is outside the verified capability profile.");
            }
            if string(map, "password").is_none_or(str::is_empty) {
                return reject("The node password is missing.");
            }
        }
        "vmess" | "vless" => {
            if protocol == "vless" && map.get("tls").and_then(Value::as_bool) != Some(true) {
                return reject("Only the VLESS TCP/TLS profile is supported.");
            }
            if protocol == "vmess" && map.contains_key("cipher") && string(map, "cipher") != Some("auto") {
                return reject("Only the encrypted VMess auto cipher profile is supported.");
            }
            if protocol == "vmess"
                && map.contains_key("alterId")
                && map.get("alterId").and_then(Value::as_u64) != Some(0)
            {
                return reject("Only the VMess AEAD profile with alterId 0 is supported.");
            }
            let Some(uuid) = string(map, "uuid") else {
                return reject("The node UUID is missing.");
            };
            if uuid.len() != 36
                || !uuid.bytes().enumerate().all(|(i, c)| {
                    if [8, 13, 18, 23].contains(&i) {
                        c == b'-'
                    } else {
                        c.is_ascii_hexdigit()
                    }
                })
            {
                return reject("The node UUID is invalid.");
            }
            if string(map, "flow").is_some_and(|flow| !flow.is_empty()) {
                return reject("This VLESS flow has not passed the capability matrix.");
            }
        }
        "trojan" | "anytls" => {
            if string(map, "password").is_none_or(str::is_empty) {
                return reject("The node password is missing.");
            }
        }
        _ => return reject("This protocol is not supported as a chain front."),
    }
    if map.iter().any(|(key, value)| {
        key.as_str()
            .is_some_and(|k| ["password", "uuid", "cipher", "sni", "servername", "client-fingerprint"].contains(&k))
            && value.as_str().is_none_or(|s| s.len() > 4096)
    }) {
        return reject("A required node field has an invalid type or length.");
    }
    None
}

#[cfg(test)]
mod tests {
    use super::{MAX_NODES, MAX_SUBSCRIPTION_BYTES, parse};

    #[test]
    fn accepts_size_and_node_limits_but_rejects_the_next_byte_or_node() -> anyhow::Result<()> {
        let mut source = "proxies: []\n#".to_owned();
        source.extend(std::iter::repeat_n('x', MAX_SUBSCRIPTION_BYTES - source.len()));
        assert!(parse(source.clone(), "limit".into(), None).is_ok());
        source.push('x');
        assert!(parse(source, "limit".into(), None).is_err());
        let node = "- {type: ss, server: example.com, port: 443, cipher: aes-128-gcm, password: fixture}\n";
        let source = format!("proxies:\n{}", node.repeat(MAX_NODES));
        let imported = parse(source.clone(), "limit".into(), None)?;
        assert_eq!(imported.duplicates, MAX_NODES - 1);
        assert!(parse(format!("{source}{node}"), "limit".into(), None).is_err());
        Ok(())
    }

    #[test]
    fn rejects_aliases_tags_deep_trees_and_provider_only_sources() {
        for source in [
            "proxies: &p []\nx: *p",
            "proxies: !custom []",
            "proxy-providers: {}",
            &format!("proxies: {}0{}", "[".repeat(33), "]".repeat(33)),
        ] {
            assert!(parse(source.into(), "test".into(), None).is_err());
        }
    }

    #[test]
    fn rejects_plaintext_vless_unverified_certificates_and_unencrypted_vmess() -> anyhow::Result<()> {
        let source = "proxies:\n- {type: vless, server: example.com, port: 443, uuid: 11111111-1111-1111-1111-111111111111, tls: false}\n- {type: vless, server: example.com, port: 443, uuid: 11111111-1111-1111-1111-111111111111, tls: true}\n- {type: vmess, server: example.com, port: 443, uuid: 11111111-1111-1111-1111-111111111111, cipher: none}\n- {type: vmess, server: example.com, port: 443, uuid: 11111111-1111-1111-1111-111111111111, cipher: auto}\n- {type: trojan, server: example.com, port: 443, password: fixture, skip-cert-verify: true}\n";
        let parsed = parse(source.into(), "capabilities".into(), None)?;
        assert_eq!(
            parsed
                .nodes
                .iter()
                .map(|node| node.rejection.is_some())
                .collect::<Vec<_>>(),
            [true, false, true, false, true]
        );
        Ok(())
    }

    #[test]
    fn identities_are_stable_across_names_and_field_order_but_keep_different_authentication() -> anyhow::Result<()> {
        let source = "proxies:\n- {name: 'same! & literal', type: ss, server: EXAMPLE.com., port: 443, cipher: aes-128-gcm, password: first}\n- {password: first, cipher: aes-128-gcm, port: 443, server: example.com, type: ss, name: renamed}\n- {name: 'same! & literal', type: ss, server: example.com, port: 443, cipher: aes-128-gcm, password: second}\n";
        let parsed = parse(source.into(), "test".into(), None)?;
        assert_eq!(parsed.nodes.len(), 2);
        assert_eq!(parsed.duplicates, 1);
        assert_ne!(parsed.nodes[0].id, parsed.nodes[1].id);
        assert!(parsed.nodes.iter().all(|node| node.rejection.is_none()));
        Ok(())
    }
}

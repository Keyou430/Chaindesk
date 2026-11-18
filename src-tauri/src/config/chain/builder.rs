use super::{ExitAuthentication, ExitEndpoint, FrontNode, RulePreset, failure};
use anyhow::Result;
use serde_yaml_ng::{Mapping, Value};

pub(crate) const FRONT_GROUP: &str = "CHAIN_DESK_FRONT";
pub(crate) const EXIT_GROUP: &str = "CHAIN_DESK_EXIT";
pub(crate) const EXIT_NODE: &str = "CHAIN_DESK_FIXED_EXIT";

pub(crate) fn front_probe(mut chain: Mapping) -> Mapping {
    if let Some(proxies) = chain.get_mut("proxies").and_then(Value::as_sequence_mut) {
        proxies.retain(|node| node["name"].as_str() != Some(EXIT_NODE));
    }
    if let Some(groups) = chain.get_mut("proxy-groups").and_then(Value::as_sequence_mut) {
        groups.retain(|group| group["name"].as_str() == Some(FRONT_GROUP));
    }
    chain.insert(
        "rules".into(),
        Value::Sequence(vec![format!("MATCH,{FRONT_GROUP}").into()]),
    );
    if let Some(dns) = chain.get_mut("dns").and_then(Value::as_mapping_mut) {
        dns.remove("nameserver-policy");
        dns.insert(
            "nameserver".into(),
            Value::Sequence(vec![format!("https://1.1.1.1/dns-query#{FRONT_GROUP}").into()]),
        );
    }
    chain
}

pub(crate) fn blocked(mixed_port: u16) -> Mapping {
    Mapping::from_iter([
        ("mixed-port".into(), mixed_port.into()),
        ("mode".into(), "rule".into()),
        (
            "dns".into(),
            Value::Mapping(Mapping::from_iter([("enable".into(), false.into())])),
        ),
        ("proxies".into(), Value::Sequence(Vec::new())),
        ("proxy-groups".into(), Value::Sequence(Vec::new())),
        ("rules".into(), Value::Sequence(vec!["MATCH,REJECT".into()])),
        ("log-level".into(), "silent".into()),
    ])
}

pub(crate) fn build(
    front: &FrontNode,
    endpoint: &ExitEndpoint,
    authentication: &ExitAuthentication,
    preset: RulePreset,
    mixed_port: u16,
) -> Result<Mapping> {
    let endpoint = endpoint.normalized()?;
    authentication.validate()?;
    if front.rejection.is_some() || endpoint.matches(&front.server, front.port) {
        return Err(failure(
            "NO_VALID_FRONT",
            "Choose an eligible front with a different endpoint from the exit.",
        ));
    }
    if mixed_port == 0 {
        return Err(failure("PORT_BUSY", "The local proxy port is not available."));
    }
    let front_name = format!("CHAIN_DESK_NODE_{}", front.id);
    let mut node = front.options.clone();
    node.insert("name".into(), front_name.clone().into());
    node.insert("udp".into(), false.into());
    let mut exit = Mapping::from_iter([
        ("name".into(), EXIT_NODE.into()),
        ("type".into(), "socks5".into()),
        ("server".into(), endpoint.host.into()),
        ("port".into(), endpoint.port.into()),
        ("udp".into(), false.into()),
        ("dialer-proxy".into(), FRONT_GROUP.into()),
    ]);
    if !authentication.username.is_empty() {
        exit.insert("username".into(), authentication.username.clone().into());
        exit.insert("password".into(), authentication.password.clone().into());
    }
    let group = |name: &str, child: String| {
        Value::Mapping(Mapping::from_iter([
            ("name".into(), name.into()),
            ("type".into(), "select".into()),
            ("proxies".into(), Value::Sequence(vec![child.into()])),
        ]))
    };
    let mut rules = [
        "DOMAIN,localhost,DIRECT",
        "DOMAIN-SUFFIX,local,DIRECT",
        "IP-CIDR,127.0.0.0/8,DIRECT,no-resolve",
        "IP-CIDR,10.0.0.0/8,DIRECT,no-resolve",
        "IP-CIDR,172.16.0.0/12,DIRECT,no-resolve",
        "IP-CIDR,192.168.0.0/16,DIRECT,no-resolve",
        "IP-CIDR,169.254.0.0/16,DIRECT,no-resolve",
    ]
    .into_iter()
    .map(Value::from)
    .collect::<Vec<_>>();
    for host in [
        "www.gstatic.com",
        "cp.cloudflare.com",
        "api.ipify.org",
        "www.cloudflare.com",
    ] {
        rules.push(format!("DOMAIN,{host},{EXIT_GROUP}").into());
    }
    if matches!(preset, RulePreset::ChinaDirect) {
        rules.extend(["GEOSITE,cn,DIRECT".into(), "GEOIP,CN,DIRECT,no-resolve".into()]);
    }
    rules.push(format!("MATCH,{EXIT_GROUP}").into());
    let mut dns = Mapping::from_iter([
        ("enable".into(), true.into()),
        ("enhanced-mode".into(), "redir-host".into()),
        ("ipv6".into(), false.into()),
        ("prefer-h3".into(), false.into()),
        (
            "default-nameserver".into(),
            Value::Sequence(vec!["223.5.5.5".into(), "119.29.29.29".into()]),
        ),
        (
            "proxy-server-nameserver".into(),
            Value::Sequence(vec!["223.5.5.5".into(), "119.29.29.29".into()]),
        ),
        (
            "direct-nameserver".into(),
            Value::Sequence(vec!["https://dns.alidns.com/dns-query#DIRECT".into()]),
        ),
        (
            "nameserver".into(),
            Value::Sequence(vec![format!("https://1.1.1.1/dns-query#{EXIT_GROUP}").into()]),
        ),
    ]);
    if matches!(preset, RulePreset::ChinaDirect) {
        dns.insert(
            "nameserver-policy".into(),
            Value::Mapping(Mapping::from_iter([(
                "geosite:cn".into(),
                Value::Sequence(vec!["https://dns.alidns.com/dns-query#DIRECT".into()]),
            )])),
        );
    }
    Ok(Mapping::from_iter([
        ("mixed-port".into(), mixed_port.into()),
        ("mode".into(), "rule".into()),
        ("ipv6".into(), false.into()),
        ("allow-lan".into(), false.into()),
        ("bind-address".into(), "127.0.0.1".into()),
        ("log-level".into(), "silent".into()),
        ("geo-auto-update".into(), false.into()),
        ("geodata-mode".into(), true.into()),
        (
            "tun".into(),
            Value::Mapping(Mapping::from_iter([("enable".into(), false.into())])),
        ),
        ("dns".into(), Value::Mapping(dns)),
        (
            "proxies".into(),
            Value::Sequence(vec![Value::Mapping(node), Value::Mapping(exit)]),
        ),
        (
            "proxy-groups".into(),
            Value::Sequence(vec![
                group(FRONT_GROUP, front_name),
                group(EXIT_GROUP, EXIT_NODE.into()),
            ]),
        ),
        ("rules".into(), Value::Sequence(rules)),
    ]))
}

#[cfg(test)]
mod tests {
    use super::{EXIT_NODE, ExitAuthentication, ExitEndpoint, RulePreset, Value, build};
    use crate::config::chain::importer;

    #[cfg(windows)]
    #[test]
    fn portable_pac_never_offers_direct_as_a_proxy_failure_fallback() {
        let pac = crate::config::DEFAULT_PAC;
        assert!(pac.contains("PROXY 127.0.0.1:%mixed-port%"));
        assert!(!pac.contains("DIRECT"));
    }

    #[test]
    fn fixes_the_exit_without_direct_fallback_or_source_configuration_inheritance() -> anyhow::Result<()> {
        let imported = importer::parse("proxies: [{name: DIRECT, type: ss, server: example.com, port: 443, cipher: aes-128-gcm, password: 'a:!&*'}]\ntun: {enable: true}\nlisteners: [untrusted]\n".into(), "test".into(), None)?;
        let endpoint = ExitEndpoint {
            host: "exit.example.com".into(),
            port: 1080,
        };
        let auth = ExitAuthentication {
            username: "u:名".into(),
            password: "p\"&*".into(),
        };
        let generated = build(&imported.nodes[0], &endpoint, &auth, RulePreset::AllProxy, 7897)?;
        assert_eq!(
            generated,
            build(&imported.nodes[0], &endpoint, &auth, RulePreset::AllProxy, 7897)?
        );
        let groups = generated["proxy-groups"]
            .as_sequence()
            .ok_or_else(|| anyhow::anyhow!("missing groups"))?;
        assert_eq!(groups[1]["proxies"], Value::Sequence(vec![EXIT_NODE.into()]));
        assert!(!generated.contains_key("listeners"));
        assert_eq!(generated["tun"]["enable"], Value::Bool(false));
        assert!(
            build(
                &imported.nodes[0],
                &ExitEndpoint {
                    host: "EXAMPLE.com.".into(),
                    port: 443
                },
                &auth,
                RulePreset::AllProxy,
                7897
            )
            .is_err()
        );
        assert!(
            ExitAuthentication {
                username: "名".repeat(86),
                password: "x".into()
            }
            .validate()
            .is_err()
        );
        Ok(())
    }
}

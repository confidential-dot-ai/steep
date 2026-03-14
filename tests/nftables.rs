use steep::nftables;

#[test]
fn test_base_rules_drops_all_input() {
    let rules = nftables::base_rules();
    assert!(rules.contains("policy drop"));
    assert!(rules.contains("chain input"));
    assert!(rules.contains("chain output"));
    assert!(rules.contains("chain forward"));
}

#[test]
fn test_base_rules_allows_loopback() {
    let rules = nftables::base_rules();
    assert!(rules.contains(r#"iif "lo" accept"#));
    assert!(rules.contains(r#"oif "lo" accept"#));
}

#[test]
fn test_base_rules_allows_established() {
    let rules = nftables::base_rules();
    assert!(rules.contains("ct state established,related accept"));
}

#[test]
fn test_base_rules_output_policy_is_drop() {
    let rules = nftables::base_rules();
    assert!(rules.contains("chain output {\n        type filter hook output priority 0; policy drop;"));
}

#[test]
fn test_service_rules_opens_port() {
    let rules = nftables::service_rules(8080);
    assert!(rules.contains("tcp dport 8080 accept"));
}

#[test]
fn test_service_rules_output_policy_is_accept() {
    let rules = nftables::service_rules(443);
    assert!(rules.contains("chain output {\n        type filter hook output priority 0; policy accept;"));
}

#[test]
fn test_service_rules_starts_with_shebang() {
    let rules = nftables::service_rules(443);
    assert!(rules.starts_with("#!/usr/sbin/nft -f\n"));
}

#[test]
fn test_base_rules_starts_with_shebang() {
    let rules = nftables::base_rules();
    assert!(rules.starts_with("#!/usr/sbin/nft -f\n"));
}

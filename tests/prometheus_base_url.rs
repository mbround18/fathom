use fathom::backend::integrations::prometheus_base_url_from_parts;

#[test]
fn explicit_client_url_wins() {
    let url = prometheus_base_url_from_parts(Some("http://10.0.0.1:9090"), &[], None);
    assert_eq!(url, Some("http://10.0.0.1:9090".to_string()));
}

#[test]
fn port_forward_remote_9090_selected() {
    let pf = vec![(3000u16, 9090u16)];
    let url = prometheus_base_url_from_parts(None, &pf, None);
    assert_eq!(url, Some("http://127.0.0.1:3000".to_string()));
}

#[test]
fn port_forward_local_9090_selected() {
    let pf = vec![(9090u16, 1234u16)];
    let url = prometheus_base_url_from_parts(None, &pf, None);
    assert_eq!(url, Some("http://127.0.0.1:9090".to_string()));
}

#[test]
fn configured_prometheus_port_used_when_no_pf_or_client() {
    let url = prometheus_base_url_from_parts(None, &[], Some(5555));
    assert_eq!(url, Some("http://127.0.0.1:5555".to_string()));
}

#[test]
fn none_when_no_evidence() {
    let url = prometheus_base_url_from_parts(None, &[], None);
    assert_eq!(url, None);
}

#[test]
fn precedence_prom_client_over_pf_and_config() {
    let pf = vec![(3000u16, 9090u16)];
    let url = prometheus_base_url_from_parts(Some("http://10.1.1.1:9090"), &pf, Some(5555));
    assert_eq!(url, Some("http://10.1.1.1:9090".to_string()));
}

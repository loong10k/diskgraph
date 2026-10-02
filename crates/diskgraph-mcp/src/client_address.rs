use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};

use crate::http::NetworkPolicy;

const MAX_FORWARDED_HOPS: usize = 32;

/// 解析限流及诊断用 IP；参数 `peer` 为直接连接地址，`headers` 为请求头，
/// `policy` 为可信代理配置。返回规范化 IP；非法 peer 返回固定 `unknown`。
/// 转发头不建立授权身份；非法或过长链回退到实际 peer。
/// 来源：DiskGraph 原生 Rust HTTP 网络策略，无对应 Java 方法。
pub fn observed_client_ip(
    peer: &str,
    headers: &HashMap<String, String>,
    policy: &NetworkPolicy,
) -> String {
    let Some(peer) = peer
        .parse::<SocketAddr>()
        .map(|address| address.ip())
        .ok()
        .or_else(|| peer.parse::<IpAddr>().ok())
        .map(canonical_ip)
    else {
        return "unknown".to_owned();
    };
    if !is_trusted(peer, policy) {
        return peer.to_string();
    }
    let Some(forwarded) = headers.get("x-forwarded-for") else {
        return peer.to_string();
    };
    let mut hops = Vec::new();
    for hop in forwarded.split(',') {
        if hops.len() >= MAX_FORWARDED_HOPS {
            return peer.to_string();
        }
        let Ok(address) = hop.trim().parse::<IpAddr>() else {
            return peer.to_string();
        };
        hops.push(canonical_ip(address));
    }
    // 可信代理须追加真实 peer；从服务端一侧回溯，只跨越显式可信 hop。
    // 第一个不可信 hop 左侧的前缀由客户端控制，不能用于刷新请求额度。
    let mut client = peer;
    for hop in hops.into_iter().rev() {
        if !is_trusted(client, policy) {
            break;
        }
        client = hop;
    }
    client.to_string()
}

fn canonical_ip(address: IpAddr) -> IpAddr {
    match address {
        IpAddr::V6(address) => address
            .to_ipv4_mapped()
            .map_or(IpAddr::V6(address), IpAddr::V4),
        address => address,
    }
}

fn is_trusted(address: IpAddr, policy: &NetworkPolicy) -> bool {
    policy.trusted_proxies.iter().any(|proxy| {
        proxy
            .parse::<IpAddr>()
            .ok()
            .is_some_and(|proxy| canonical_ip(proxy) == address)
    })
}

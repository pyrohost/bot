use reqwest::Client;
use serde::Deserialize;
use std::time::Duration;
use std::collections::HashMap;

use crate::error::BotError;

const API_TIMEOUT: Duration = Duration::from_secs(5);
const API_ENDPOINT: &str = "https://metrics.pyro.host/api/v1/query";

#[derive(Debug)]
pub struct Metric {
    pub name: &'static str,
    pub icon: &'static str,
    pub query: &'static str,
    pub format: fn(f64) -> String,
}

impl Metric {
    const fn new(
        name: &'static str,
        icon: &'static str,
        query: &'static str,
        format: fn(f64) -> String,
    ) -> Self {
        Self {
            name,
            icon,
            query,
            format,
        }
    }

    pub fn format_value(&self, value: f64) -> String {
        format!("{} {}", self.icon, (self.format)(value))
    }
}

#[derive(Debug, Deserialize)]
struct PrometheusResponse {
    status: String,
    data: PrometheusData,
}

#[derive(Debug, Deserialize)]
struct PrometheusData {
    result: Vec<PrometheusResult>,
}

#[derive(Debug, Deserialize)]
struct PrometheusResult {
    metric: HashMap<String, String>,
    value: (f64, serde_json::Value),
}

#[derive(Debug)]
pub struct MetricsClient {
    client: Client,
}

impl MetricsClient {
    pub fn new() -> Self {
        Self {
            client: Client::builder()
                .timeout(API_TIMEOUT)
                .build()
                .expect("Failed to create HTTP client"),
        }
    }

    pub async fn fetch_metric(&self, query: &str) -> crate::error::Result<f64> {
        let resp: PrometheusResponse = self
            .client
            .get(API_ENDPOINT)
            .query(&[("query", query)])
            .send()
            .await
            .map_err(BotError::Http)?
            .json()
            .await
            .map_err(BotError::Http)?;

        match resp {
            PrometheusResponse {
                status,
                data: PrometheusData { result },
            } if status == "success" => result
                .first()
                .and_then(|r| r.value.1.as_str().and_then(|s| s.parse().ok()))
                .ok_or_else(|| BotError::Metrics("No valid metric value found".to_string())),
            _ => Err(BotError::Metrics("Prometheus query failed".to_string())),
        }
    }

    pub async fn fetch_existing_trees(&self) -> crate::error::Result<Vec<String>> {
        let query = "node_uname_info";
        let resp: PrometheusResponse = self
            .client
            .get(API_ENDPOINT)
            .query(&[("query", query)])
            .send()
            .await
            .map_err(BotError::Http)?
            .json()
            .await
            .map_err(BotError::Http)?;

        let mut node_names = Vec::new();
        let PrometheusResponse {
            status,
            data: PrometheusData { result },
        } = resp;
        
        if status == "success" {
            for item in result {
                if let Some(nodename) = item.metric.get("nodename") {
                    node_names.push(nodename.clone());
                }
            }
        }
        
        Ok(node_names)
    }
}

pub const METRICS: &[Metric] = &[
    Metric::new(
        "nodes",
        "🖥️",
        "count(up{job=\"node\"} == 1)",
        |v| format!("Active Nodes {:.0}", v),
    ),
    Metric::new(
        "network",
        "🌐",
        "sum(rate(node_network_receive_bytes_total[5m]) + rate(node_network_transmit_bytes_total[5m])) or vector(0)",
        |v| format!("Net {}/s", format_bytes(v)),
    ),
    Metric::new(
        "network_total",
        "📊",
        "sum(increase(node_network_receive_bytes_total[7d]) + increase(node_network_transmit_bytes_total[7d])) or vector(0)",
        |v| format!("7d Total {}", format_large_bytes(v)),
    ),
    Metric::new(
        "storage",
        "💾",
        "sum(node_filesystem_size_bytes{mountpoint=\"/\"} - node_filesystem_free_bytes{mountpoint=\"/\"})",
        |v| format!("Disk {}", format_bytes(v)),
    ),
    Metric::new(
        "memory",
        "🧠",
        "sum(node_memory_MemTotal_bytes - node_memory_MemAvailable_bytes) or vector(0)",
        |v| format!("Mem {}", format_bytes(v)),
    ),
];

pub fn format_bytes(bytes: f64) -> String {
    const UNITS: [&str; 6] = ["B", "KB", "MB", "GB", "TB", "PB"];
    let mut value = bytes;
    let mut unit_index = 0;

    while value >= 1024.0 && unit_index < UNITS.len() - 1 {
        value /= 1024.0;
        unit_index += 1;
    }

    if value >= 100.0 {
        format!("{:.1}{}", value, UNITS[unit_index])
    } else {
        format!("{:.2}{}", value, UNITS[unit_index])
    }
}

pub fn format_large_bytes(bytes: f64) -> String {
    const UNITS: [&str; 6] = ["B", "KB", "MB", "GB", "TB", "PB"];
    let mut value = bytes;
    let mut unit_index = 0;

    while value >= 1024.0 && unit_index < UNITS.len() - 1 {
        value /= 1024.0;
        unit_index += 1;
    }

    format!("{:.1}{}", value, UNITS[unit_index])
}

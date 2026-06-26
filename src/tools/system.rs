//! High-value system tools with robust UI finding and metrics.

use std::process::Command;
use std::time::Instant;

use metrics::{counter, histogram};
use rmcp::tool;
use roxmltree::Document;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::process::Command as TokioCommand;

use crate::error::AppError;

#[derive(Clone, Default)]
pub struct SystemTools;

#[derive(Debug, Serialize, Deserialize)]
pub struct SensorReading {
    pub sensor: String,
    pub values: Vec<f32>,
    pub accuracy: Option<i32>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct LogcatResult {
    pub lines: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct CommandResult {
    pub stdout: String,
    pub stderr: String,
    pub exit_code: i32,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct UiElement {
    pub text: Option<String>,
    pub resource_id: Option<String>,
    pub class: Option<String>,
    pub package: Option<String>,
    pub bounds: Option<String>,
    pub clickable: bool,
    pub focusable: bool,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct UiDumpResult {
    pub elements: Vec<UiElement>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct UiQueryResult {
    pub elements: Vec<UiElement>,
    pub count: usize,
}

// ==================== IMPLEMENTATIONS ====================

#[tool]
impl SystemTools {
    #[tool(description = "Read sensor with rich structured data")]
    pub async fn read_sensor(&self, sensor: String) -> Result<SensorReading, AppError> {
        let start = Instant::now();
        let output = TokioCommand::new("termux-sensor")
            .args(["-s", &sensor, "-n", "1"])
            .output()
            .await?;

        let duration = start.elapsed().as_secs_f64();
        histogram!("mcp.sensor.latency_seconds").record(duration);
        counter!("mcp.sensor.calls_total").increment(1);

        let stdout = String::from_utf8_lossy(&output.stdout);
        let (values, accuracy) = parse_sensor_json(&stdout, &sensor);

        Ok(SensorReading { sensor, values, accuracy })
    }

    #[tool(description = "Get recent logcat")]
    pub async fn get_logcat(&self, lines: Option<u32>) -> Result<LogcatResult, AppError> {
        let start = Instant::now();
        let count = lines.unwrap_or(100);
        let output = Command::new("logcat").args(["-d", "-t", &count.to_string()]).output()?;
        let duration = start.elapsed().as_secs_f64();
        histogram!("mcp.logcat.latency_seconds").record(duration);
        let stdout = String::from_utf8_lossy(&output.stdout);
        Ok(LogcatResult { lines: stdout.lines().map(|s| s.to_string()).collect() })
    }

    #[tool(description = "Execute via rish with metrics")]
    pub async fn rish_exec(&self, command: String) -> Result<CommandResult, AppError> {
        let start = Instant::now();
        let output = TokioCommand::new("rish").arg("-c").arg(&command).output().await?;
        let duration = start.elapsed().as_secs_f64();
        histogram!("mcp.rish.latency_seconds").record(duration);
        counter!("mcp.rish.calls_total").increment(1);
        if !output.status.success() { counter!("mcp.rish.errors_total").increment(1); }
        Ok(CommandResult {
            stdout: String::from_utf8_lossy(&output.stdout).to_string(),
            stderr: String::from_utf8_lossy(&output.stderr).to_string(),
            exit_code: output.status.code().unwrap_or(-1),
        })
    }

    #[tool(description = "Dump UI hierarchy (parsed)")]
    pub async fn dump_ui_hierarchy(&self) -> Result<UiDumpResult, AppError> {
        let start = Instant::now();
        let cmd = "uiautomator dump /sdcard/window_dump.xml 2>/dev/null && cat /sdcard/window_dump.xml";
        let output = TokioCommand::new("rish").arg("-c").arg(cmd).output().await?;
        let xml = String::from_utf8_lossy(&output.stdout).to_string();
        let duration = start.elapsed().as_secs_f64();
        histogram!("mcp.ui.latency_seconds").record(duration);
        let elements = parse_ui_xml(&xml);
        Ok(UiDumpResult { elements })
    }

    #[tool(description = "Find elements by resource ID (robust)")]
    pub async fn find_elements_by_resource_id(&self, resource_id: String) -> Result<UiQueryResult, AppError> {
        let dump = self.dump_ui_hierarchy().await?;
        let search = resource_id.to_lowercase();
        let elements: Vec<UiElement> = dump.elements.into_iter()
            .filter(|e| e.resource_id.as_ref().map_or(false, |id| id.to_lowercase().contains(&search)))
            .collect();
        Ok(UiQueryResult { count: elements.len(), elements })
    }

    #[tool(description = "Find elements by class")]
    pub async fn find_elements_by_class(&self, class_name: String) -> Result<UiQueryResult, AppError> {
        let dump = self.dump_ui_hierarchy().await?;
        let elements: Vec<UiElement> = dump.elements.into_iter()
            .filter(|e| e.class.as_ref().map_or(false, |c| c.contains(&class_name)))
            .collect();
        Ok(UiQueryResult { count: elements.len(), elements })
    }

    #[tool(description = "Find element by text (case-insensitive contains)")]
    pub async fn find_element_by_text(&self, text: String) -> Result<Option<UiElement>, AppError> {
        let dump = self.dump_ui_hierarchy().await?;
        let search = text.to_lowercase();
        let found = dump.elements.into_iter().find(|e| {
            e.text.as_ref().map_or(false, |t| t.to_lowercase().contains(&search))
        });
        Ok(found)
    }

    #[tool(description = "Find element by text and return center tap coordinates")]
    pub async fn find_element_and_get_tap_coordinates(&self, text: String) -> Result<Option<(i32, i32)>, AppError> {
        if let Some(element) = self.find_element_by_text(text).await? {
            if let Some(bounds) = &element.bounds {
                if let Some((x1, y1, x2, y2)) = parse_bounds(bounds) {
                    return Ok(Some(((x1 + x2) / 2, (y1 + y2) / 2)));
                }
            }
        }
        Ok(None)
    }
}

// ==================== Helpers ====================

fn parse_sensor_json(stdout: &str, sensor: &str) -> (Vec<f32>, Option<i32>) {
    if let Ok(json) = serde_json::from_str::<Value>(stdout) {
        if let Some(data) = json.get(sensor) {
            let values: Vec<f32> = data.get("values")
                .and_then(|v| v.as_array())
                .map(|arr| arr.iter().filter_map(|x| x.as_f64().map(|f| f as f32)).collect())
                .unwrap_or_default();
            let accuracy = data.get("accuracy").and_then(|a| a.as_i64()).map(|i| i as i32);
            return (values, accuracy);
        }
    }
    (vec![], None)
}

fn parse_ui_xml(xml: &str) -> Vec<UiElement> {
    let mut elements = Vec::new();
    if let Ok(doc) = Document::parse(xml) {
        for node in doc.descendants() {
            if node.has_tag_name("node") {
                elements.push(UiElement {
                    text: node.attribute("text").map(|s| s.to_string()),
                    resource_id: node.attribute("resource-id").map(|s| s.to_string()),
                    class: node.attribute("class").map(|s| s.to_string()),
                    package: node.attribute("package").map(|s| s.to_string()),
                    bounds: node.attribute("bounds").map(|s| s.to_string()),
                    clickable: node.attribute("clickable").map_or(false, |v| v == "true"),
                    focusable: node.attribute("focusable").map_or(false, |v| v == "true"),
                });
            }
        }
    }
    elements
}

fn parse_bounds(bounds: &str) -> Option<(i32, i32, i32, i32)> {
    let cleaned = bounds.replace(['[', ']'], " ");
    let parts: Vec<&str> = cleaned.split_whitespace().collect();
    if parts.len() == 2 {
        let left_top: Vec<i32> = parts[0].split(',').filter_map(|s| s.parse().ok()).collect();
        let right_bottom: Vec<i32> = parts[1].split(',').filter_map(|s| s.parse().ok()).collect();
        if left_top.len() == 2 && right_bottom.len() == 2 {
            return Some((left_top[0], left_top[1], right_bottom[0], right_bottom[1]));
        }
    }
    None
}

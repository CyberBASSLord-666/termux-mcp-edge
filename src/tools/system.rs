//! High-value system tools with robust UI finding and metrics.

use std::time::Instant;

use metrics::{counter, histogram};
use roxmltree::Document;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::process::Command as TokioCommand;

use crate::error::AppError;

const UI_DUMP_PATH: &str = "/sdcard/window_dump.xml";

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

impl SystemTools {
    pub async fn read_sensor(&self, sensor: String) -> Result<SensorReading, AppError> {
        let start = Instant::now();
        let output = TokioCommand::new("termux-sensor")
            .args(["-s", &sensor, "-n", "1"])
            .output()
            .await?;

        let duration = start.elapsed().as_secs_f64();
        histogram!("mcp.sensor.latency_seconds").record(duration);
        counter!("mcp.sensor.calls_total").increment(1);

        ensure_success("termux-sensor", &output)?;
        let stdout = String::from_utf8_lossy(&output.stdout);
        let (values, accuracy) = parse_sensor_json(&stdout, &sensor);

        Ok(SensorReading {
            sensor,
            values,
            accuracy,
        })
    }
    pub async fn get_logcat(&self, lines: Option<u32>) -> Result<LogcatResult, AppError> {
        let start = Instant::now();
        let count = lines.unwrap_or(100);
        let output = TokioCommand::new("logcat")
            .args(["-d", "-t", &count.to_string()])
            .output()
            .await?;
        ensure_success("logcat", &output)?;
        let duration = start.elapsed().as_secs_f64();
        histogram!("mcp.logcat.latency_seconds").record(duration);
        let stdout = String::from_utf8_lossy(&output.stdout);

        Ok(LogcatResult {
            lines: stdout.lines().map(str::to_string).collect(),
        })
    }
    pub async fn rish_exec(&self, command: String) -> Result<CommandResult, AppError> {
        let start = Instant::now();
        let output = TokioCommand::new("rish")
            .arg("-c")
            .arg(&command)
            .output()
            .await?;
        let duration = start.elapsed().as_secs_f64();
        histogram!("mcp.rish.latency_seconds").record(duration);
        counter!("mcp.rish.calls_total").increment(1);
        if !output.status.success() {
            counter!("mcp.rish.errors_total").increment(1);
        }

        Ok(CommandResult {
            stdout: String::from_utf8_lossy(&output.stdout).to_string(),
            stderr: String::from_utf8_lossy(&output.stderr).to_string(),
            exit_code: output.status.code().unwrap_or(-1),
        })
    }
    pub async fn dump_ui_hierarchy(&self) -> Result<UiDumpResult, AppError> {
        let start = Instant::now();
        let dump_command = format!("uiautomator dump {UI_DUMP_PATH}");
        let dump_output = TokioCommand::new("rish")
            .arg("-c")
            .arg(&dump_command)
            .output()
            .await?;
        if !dump_output.status.success() {
            counter!("mcp.ui.dump_errors_total").increment(1);
            return Err(command_failure("uiautomator dump", &dump_output));
        }

        let read_command = format!("cat {UI_DUMP_PATH}");
        let output = TokioCommand::new("rish")
            .arg("-c")
            .arg(&read_command)
            .output()
            .await?;
        ensure_success("cat UI hierarchy dump", &output)?;
        let _ = tokio::fs::remove_file(UI_DUMP_PATH).await;
        let xml = String::from_utf8_lossy(&output.stdout).to_string();
        let duration = start.elapsed().as_secs_f64();
        histogram!("mcp.ui.latency_seconds").record(duration);
        let elements = parse_ui_xml(&xml);

        Ok(UiDumpResult { elements })
    }
    pub async fn find_elements_by_resource_id(
        &self,
        resource_id: String,
    ) -> Result<UiQueryResult, AppError> {
        let dump = self.dump_ui_hierarchy().await?;
        let search = resource_id.to_lowercase();
        let elements: Vec<UiElement> = dump
            .elements
            .into_iter()
            .filter(|element| {
                element
                    .resource_id
                    .as_ref()
                    .is_some_and(|id| id.to_lowercase().contains(&search))
            })
            .collect();

        Ok(UiQueryResult {
            count: elements.len(),
            elements,
        })
    }
    pub async fn find_elements_by_class(
        &self,
        class_name: String,
    ) -> Result<UiQueryResult, AppError> {
        let dump = self.dump_ui_hierarchy().await?;
        let elements: Vec<UiElement> = dump
            .elements
            .into_iter()
            .filter(|element| {
                element
                    .class
                    .as_ref()
                    .is_some_and(|class| class.contains(&class_name))
            })
            .collect();

        Ok(UiQueryResult {
            count: elements.len(),
            elements,
        })
    }
    pub async fn find_element_by_text(&self, text: String) -> Result<Option<UiElement>, AppError> {
        let dump = self.dump_ui_hierarchy().await?;
        let search = text.to_lowercase();
        let found = dump.elements.into_iter().find(|element| {
            element
                .text
                .as_ref()
                .is_some_and(|candidate| candidate.to_lowercase().contains(&search))
        });

        Ok(found)
    }
    pub async fn find_element_and_get_tap_coordinates(
        &self,
        text: String,
    ) -> Result<Option<(i32, i32)>, AppError> {
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

fn parse_sensor_json(stdout: &str, sensor: &str) -> (Vec<f32>, Option<i32>) {
    if let Ok(json) = serde_json::from_str::<Value>(stdout) {
        if let Some(data) = json.get(sensor) {
            let values: Vec<f32> = data
                .get("values")
                .and_then(|value| value.as_array())
                .map(|values| {
                    values
                        .iter()
                        .filter_map(|value| value.as_f64().map(|float| float as f32))
                        .collect()
                })
                .unwrap_or_default();
            let accuracy = data
                .get("accuracy")
                .and_then(|accuracy| accuracy.as_i64())
                .map(|accuracy| accuracy as i32);

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
                    text: node.attribute("text").map(str::to_string),
                    resource_id: node.attribute("resource-id").map(str::to_string),
                    class: node.attribute("class").map(str::to_string),
                    package: node.attribute("package").map(str::to_string),
                    bounds: node.attribute("bounds").map(str::to_string),
                    clickable: node.attribute("clickable") == Some("true"),
                    focusable: node.attribute("focusable") == Some("true"),
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
        let left_top: Vec<i32> = parts[0]
            .split(',')
            .filter_map(|value| value.parse().ok())
            .collect();
        let right_bottom: Vec<i32> = parts[1]
            .split(',')
            .filter_map(|value| value.parse().ok())
            .collect();

        if left_top.len() == 2 && right_bottom.len() == 2 {
            return Some((left_top[0], left_top[1], right_bottom[0], right_bottom[1]));
        }
    }

    None
}

fn ensure_success(command: &str, output: &std::process::Output) -> Result<(), AppError> {
    if output.status.success() {
        Ok(())
    } else {
        Err(command_failure(command, output))
    }
}

fn command_failure(command: &str, output: &std::process::Output) -> AppError {
    AppError::CommandFailed {
        command: command.to_string(),
        exit_code: output.status.code().unwrap_or(-1),
        stderr: String::from_utf8_lossy(&output.stderr).trim().to_string(),
    }
}

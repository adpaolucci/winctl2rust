use std::sync::Arc;

use guap_mcp::{ActionCommand, ActionRiskLevel, McpToolError};
use guap_table::{
    CellValue, ColumnDef, ColumnType, Row, TableData, TableError, TableProvider, TableQuery, TableSchema, apply_query,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use winctl2rust_core::{WinctlError, WinctlResult, dry_run_preview, validate_r4_compliance};

use crate::backend::{AudioBackend, ListenerOptions, PointerWheelEvent, ScreenEdgeKind, WheelCondition, WheelRule};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WheelEvalMatch {
    pub rule_id: String,
    pub target_device: String,
    pub previous_percent: f32,
    pub new_percent: f32,
    pub wheel_delta: i16,
    pub consume_wheel: bool,
}

pub fn default_wheel_rules() -> Vec<WheelRule> {
    vec![
        WheelRule {
            rule_id: "ctrl_alt_fine".to_string(),
            enabled: true,
            priority: 200,
            conditions: vec![WheelCondition::Ctrl, WheelCondition::Alt],
            target_device: "DefaultRenderDevice".to_string(),
            step_percent: 1.0,
            min_percent: 0.0,
            max_percent: 100.0,
            consume_wheel: false,
            invert_direction: false,
        },
        WheelRule {
            rule_id: "alt_wheel_master".to_string(),
            enabled: true,
            priority: 100,
            conditions: vec![WheelCondition::Alt],
            target_device: "DefaultRenderDevice".to_string(),
            step_percent: 5.0,
            min_percent: 0.0,
            max_percent: 100.0,
            consume_wheel: false,
            invert_direction: false,
        },
        WheelRule {
            rule_id: "over_taskbar_wheel".to_string(),
            enabled: true,
            priority: 80,
            conditions: vec![WheelCondition::OverTaskbar],
            target_device: "DefaultRenderDevice".to_string(),
            step_percent: 2.0,
            min_percent: 10.0,
            max_percent: 80.0,
            consume_wheel: false,
            invert_direction: false,
        },
        WheelRule {
            rule_id: "left_edge_headphones".to_string(),
            enabled: true,
            priority: 60,
            conditions: vec![WheelCondition::ScreenEdge(ScreenEdgeKind::Left)],
            target_device: "Headphones (USB Audio)".to_string(),
            step_percent: 4.0,
            min_percent: 0.0,
            max_percent: 100.0,
            consume_wheel: false,
            invert_direction: false,
        },
    ]
}

pub fn validate_rule(rule: &WheelRule) -> WinctlResult<()> {
    if rule.rule_id.trim().is_empty() {
        return Err(WinctlError::InvalidInput("rule_id must not be empty".to_string()));
    }
    validate_r4_compliance(&rule.rule_id, std::slice::from_ref(&rule.target_device))?;
    if !rule.step_percent.is_finite() || rule.step_percent <= 0.0 || rule.step_percent > 100.0 {
        return Err(WinctlError::InvalidInput(format!(
            "rule '{}' step_percent {} must be in (0.0, 100.0]",
            rule.rule_id, rule.step_percent
        )));
    }
    if !rule.min_percent.is_finite()
        || !rule.max_percent.is_finite()
        || rule.min_percent < 0.0
        || rule.max_percent > 100.0
        || rule.min_percent > rule.max_percent
    {
        return Err(WinctlError::InvalidInput(format!(
            "rule '{}' min/max bounds [{}, {}] invalid (must satisfy 0 <= min <= max <= 100)",
            rule.rule_id, rule.min_percent, rule.max_percent
        )));
    }
    Ok(())
}

pub fn condition_matches(cond: &WheelCondition, ev: &PointerWheelEvent) -> bool {
    match cond {
        WheelCondition::Any => true,
        WheelCondition::Alt => ev.alt,
        WheelCondition::Ctrl => ev.ctrl,
        WheelCondition::Shift => ev.shift,
        WheelCondition::Win => ev.win,
        WheelCondition::LeftButton => ev.left_button,
        WheelCondition::RightButton => ev.right_button,
        WheelCondition::MiddleButton => ev.middle_button,
        WheelCondition::OverTaskbar => ev.over_taskbar,
        WheelCondition::OverDesktop => ev.over_desktop,
        WheelCondition::OverWindowTitle => ev.over_window_title,
        WheelCondition::ScreenEdge(edge) => ev.screen_edge == Some(*edge),
        WheelCondition::WindowClassContains(needle) => {
            !needle.is_empty()
                && ev
                    .window_class
                    .to_ascii_lowercase()
                    .contains(&needle.to_ascii_lowercase())
        }
    }
}

pub fn sort_rules_by_priority(rules: &[WheelRule]) -> Vec<WheelRule> {
    let mut sorted = rules.to_vec();
    sorted.sort_by(|a, b| {
        b.priority
            .cmp(&a.priority)
            .then_with(|| b.conditions.len().cmp(&a.conditions.len()))
            .then_with(|| a.rule_id.cmp(&b.rule_id))
    });
    sorted
}

pub fn is_rule_valid_for_eval(rule: &WheelRule) -> bool {
    rule.step_percent.is_finite()
        && rule.step_percent > 0.0
        && rule.min_percent.is_finite()
        && rule.max_percent.is_finite()
        && rule.min_percent <= rule.max_percent
}

pub fn find_matching_rule(rules: &[WheelRule], event: &PointerWheelEvent) -> Option<WheelRule> {
    if event.wheel_delta == 0 {
        return None;
    }
    let ordered = sort_rules_by_priority(rules);
    for rule in ordered {
        if !rule.enabled || rule.conditions.is_empty() || !is_rule_valid_for_eval(&rule) {
            continue;
        }
        if rule.conditions.iter().all(|c| condition_matches(c, event)) {
            return Some(rule);
        }
    }
    None
}

pub fn apply_wheel_rule(
    rule: &WheelRule,
    event: &PointerWheelEvent,
    current_volume_percent: f32,
) -> Option<WheelEvalMatch> {
    if event.wheel_delta == 0 || !current_volume_percent.is_finite() || !is_rule_valid_for_eval(rule) {
        return None;
    }
    let notches = f32::from(event.wheel_delta) / 120.0;
    let sign = if rule.invert_direction { -1.0 } else { 1.0 };
    let raw_next = current_volume_percent + sign * notches * rule.step_percent;
    if !raw_next.is_finite() {
        return None;
    }
    let new_percent = raw_next.clamp(rule.min_percent, rule.max_percent);
    let rounded = (new_percent * 100.0).round() / 100.0;
    Some(WheelEvalMatch {
        rule_id: rule.rule_id.clone(),
        target_device: rule.target_device.clone(),
        previous_percent: current_volume_percent,
        new_percent: rounded,
        wheel_delta: event.wheel_delta,
        consume_wheel: rule.consume_wheel,
    })
}

pub fn evaluate_wheel_event(
    rules: &[WheelRule],
    event: &PointerWheelEvent,
    current_volume_percent: f32,
) -> Option<WheelEvalMatch> {
    if !current_volume_percent.is_finite() {
        return None;
    }
    let rule = find_matching_rule(rules, event)?;
    apply_wheel_rule(&rule, event, current_volume_percent)
}

pub fn evaluate_event_stream(
    rules: &[WheelRule],
    events: &[PointerWheelEvent],
    initial_volume_percent: f32,
) -> Vec<WheelEvalMatch> {
    if !initial_volume_percent.is_finite() {
        return Vec::new();
    }
    let mut current = initial_volume_percent.clamp(0.0, 100.0);
    let mut out = Vec::new();
    for ev in events {
        if let Some(m) = evaluate_wheel_event(rules, ev, current) {
            current = m.new_percent;
            out.push(m);
        }
    }
    out
}

pub struct WheelRulesTableProvider {
    rules: Vec<WheelRule>,
}

impl Default for WheelRulesTableProvider {
    fn default() -> Self {
        Self {
            rules: default_wheel_rules(),
        }
    }
}

impl WheelRulesTableProvider {
    pub fn with_rules(rules: Vec<WheelRule>) -> Self {
        Self { rules }
    }
}

impl TableProvider for WheelRulesTableProvider {
    fn tool_id(&self) -> &'static str {
        "audio_wheel_rules"
    }

    fn description(&self) -> &'static str {
        "Inspect mouse wheel and modifier volume rules (in-process WM_INPUT engine)"
    }

    fn schema(&self) -> TableSchema {
        TableSchema::new(
            "audio_wheel_rules",
            "Mouse Wheel Volume Rules",
            vec![
                ColumnDef::new("rule_id", "Rule ID", ColumnType::Text).with_width_hint(160),
                ColumnDef::new("enabled", "Enabled", ColumnType::Bool).with_width_hint(70),
                ColumnDef::new("priority", "Priority", ColumnType::Int).with_width_hint(80),
                ColumnDef::new("conditions", "Conditions", ColumnType::Text).with_width_hint(220),
                ColumnDef::new("target_device", "Target Device", ColumnType::Text).with_width_hint(200),
                ColumnDef::new("step_percent", "Step %", ColumnType::Float).with_width_hint(80),
                ColumnDef::new("min_percent", "Min %", ColumnType::Float).with_width_hint(80),
                ColumnDef::new("max_percent", "Max %", ColumnType::Float).with_width_hint(80),
                ColumnDef::new("invert_direction", "Invert", ColumnType::Bool).with_width_hint(70),
            ],
        )
    }

    fn is_read_only(&self) -> bool {
        true
    }

    fn query(&self, req: &TableQuery) -> Result<TableData, TableError> {
        let sorted = sort_rules_by_priority(&self.rules);
        let rows: Vec<Row> = sorted
            .into_iter()
            .enumerate()
            .map(|(idx, r)| {
                let cond_str = r
                    .conditions
                    .iter()
                    .map(|c| format!("{c:?}"))
                    .collect::<Vec<_>>()
                    .join(" + ");
                Row::new(
                    idx as u64 + 1,
                    vec![
                        CellValue::Text(r.rule_id.clone()),
                        CellValue::Bool(r.enabled),
                        CellValue::Int(i64::from(r.priority)),
                        CellValue::Text(cond_str),
                        CellValue::Text(r.target_device.clone()),
                        CellValue::Float(f64::from(r.step_percent)),
                        CellValue::Float(f64::from(r.min_percent)),
                        CellValue::Float(f64::from(r.max_percent)),
                        CellValue::Bool(r.invert_direction),
                    ],
                )
                .with_detail_json(serde_json::to_value(&r).unwrap_or(Value::Null))
            })
            .collect();
        apply_query(&self.schema(), &rows, req)
    }
}

#[derive(Debug, Clone, Deserialize)]
struct WheelRequest {
    #[serde(default = "default_mode")]
    mode: String,
    #[serde(default)]
    rules: Option<Vec<WheelRule>>,
    #[serde(default)]
    events: Vec<PointerWheelEvent>,
    #[serde(default)]
    initial_volume_percent: Option<f32>,
    #[serde(default)]
    apply_to_backend: bool,
    #[serde(default)]
    duration_ms: Option<u64>,
    #[serde(default)]
    max_events: Option<usize>,
}

fn default_mode() -> String {
    "evaluate_events".to_string()
}

pub struct WheelAction {
    backend: Arc<dyn AudioBackend>,
}

impl WheelAction {
    pub fn new(backend: Arc<dyn AudioBackend>) -> Self {
        Self { backend }
    }
}

impl ActionCommand for WheelAction {
    fn command_id(&self) -> &'static str {
        "audio_wheel"
    }

    fn description(&self) -> &'static str {
        "Evaluate or apply mouse wheel and modifier volume rules via in-process WM_INPUT (no DLL injection)"
    }

    fn risk_level(&self) -> ActionRiskLevel {
        ActionRiskLevel::Mutating
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "mode": {
                    "type": "string",
                    "enum": ["evaluate_events", "apply_rule", "start_listener", "validate_rules"],
                    "default": "evaluate_events"
                },
                "rules": { "type": "array" },
                "events": { "type": "array" },
                "initial_volume_percent": { "type": "number", "minimum": 0.0, "maximum": 100.0 },
                "apply_to_backend": { "type": "boolean", "default": false },
                "duration_ms": { "type": "integer", "minimum": 1, "maximum": 60000 },
                "max_events": { "type": "integer", "minimum": 1, "maximum": 1000 }
            }
        })
    }

    fn execute(&self, args: &Value, dry_run: bool) -> Result<Value, McpToolError> {
        let req: WheelRequest = serde_json::from_value(args.clone())
            .map_err(|e| McpToolError::InvalidParams(format!("invalid audio_wheel request: {e}")))?;
        let rules = req.rules.unwrap_or_else(default_wheel_rules);
        for r in &rules {
            validate_rule(r).map_err(McpToolError::from)?;
        }

        let mode = req.mode.trim().to_ascii_lowercase();
        match mode.as_str() {
            "validate_rules" => Ok(json!({
                "dry_run": dry_run,
                "applied": false,
                "valid": true,
                "rule_count": rules.len(),
            })),
            "evaluate_events" | "apply_rule" => {
                let init_vol = req.initial_volume_percent.unwrap_or(50.0);
                let matches = evaluate_event_stream(&rules, &req.events, init_vol);
                let should_apply = mode == "apply_rule" || req.apply_to_backend;
                if dry_run || !should_apply {
                    return Ok(if dry_run {
                        dry_run_preview(
                            self.command_id(),
                            "Preview wheel volume rule evaluation without mutating CoreAudio endpoints",
                            json!({
                                "mode": mode,
                                "initial_volume_percent": init_vol,
                                "matches": matches,
                            }),
                        )
                    } else {
                        json!({
                            "dry_run": false,
                            "applied": false,
                            "matches": matches,
                        })
                    });
                }

                let mut applied_results = Vec::new();
                for m in &matches {
                    let target = if m.target_device.is_empty() {
                        "DefaultRenderDevice"
                    } else {
                        m.target_device.as_str()
                    };
                    let snap = self
                        .backend
                        .set_volume_scalar(target, m.new_percent / 100.0)
                        .map_err(McpToolError::from)?;
                    applied_results.push(json!({
                        "rule_id": m.rule_id,
                        "target_device": target,
                        "new_percent": snap.volume_percent,
                        "muted": snap.muted,
                    }));
                }
                Ok(json!({
                    "dry_run": false,
                    "applied": !applied_results.is_empty(),
                    "matches": matches,
                    "applied_results": applied_results,
                }))
            }
            "start_listener" => {
                let opts = ListenerOptions {
                    duration_ms: req.duration_ms.unwrap_or(1000).clamp(1, 60_000),
                    max_events: req.max_events.unwrap_or(16).clamp(1, 1000),
                };
                if dry_run {
                    return Ok(dry_run_preview(
                        self.command_id(),
                        "Preview in-process WM_INPUT wheel listener startup",
                        json!({
                            "listener_api": "WM_INPUT (RegisterRawInputDevices RIDEV_INPUTSINK)",
                            "duration_ms": opts.duration_ms,
                            "max_events": opts.max_events,
                            "rule_count": rules.len(),
                        }),
                    ));
                }
                let results = self
                    .backend
                    .run_wheel_listener(&rules, &opts)
                    .map_err(McpToolError::from)?;
                Ok(json!({
                    "dry_run": false,
                    "applied": !results.is_empty(),
                    "events_handled": results.len(),
                    "results": results,
                }))
            }
            other => Err(McpToolError::InvalidParams(format!(
                "unsupported audio_wheel mode '{other}'"
            ))),
        }
    }
}

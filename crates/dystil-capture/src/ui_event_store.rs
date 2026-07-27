use crate::a11y::{EventData, UiEvent};
use chrono::{DateTime, Utc};
use sqlx::SqlitePool;

use crate::capture_store::sanitize_text;

#[derive(Debug, Clone)]
pub struct UiEventRecord {
    pub timestamp: DateTime<Utc>,
    pub session_id: String,
    pub relative_ms: i64,
    pub event_type: &'static str,
    pub x: Option<i32>,
    pub y: Option<i32>,
    pub delta_x: Option<i16>,
    pub delta_y: Option<i16>,
    pub button: Option<u8>,
    pub click_count: Option<u8>,
    pub key_code: Option<u16>,
    pub modifiers: Option<u8>,
    pub text_content: Option<String>,
    pub app_name: Option<String>,
    pub app_pid: Option<i32>,
    pub window_title: Option<String>,
    pub browser_url: Option<String>,
    pub element_role: Option<String>,
    pub element_name: Option<String>,
    pub element_value: Option<String>,
    pub element_description: Option<String>,
    pub element_automation_id: Option<String>,
    pub element_bounds: Option<String>,
    pub frame_id: Option<i64>,
}

fn clean(value: Option<String>) -> Option<String> {
    value.map(|value| sanitize_text(&value))
}

impl UiEventRecord {
    pub fn from_native(event: UiEvent, session_id: String) -> Self {
        let mut record = Self {
            timestamp: event.timestamp,
            session_id,
            relative_ms: event.relative_ms as i64,
            event_type: event.event_type(),
            x: None,
            y: None,
            delta_x: None,
            delta_y: None,
            button: None,
            click_count: None,
            key_code: None,
            modifiers: None,
            text_content: None,
            app_name: event.app_name,
            app_pid: None,
            window_title: event.window_title,
            browser_url: event.browser_url,
            element_role: event.element.as_ref().map(|e| e.role.clone()),
            element_name: event.element.as_ref().and_then(|e| e.name.clone()),
            element_value: event.element.as_ref().and_then(|e| e.value.clone()),
            element_description: event.element.as_ref().and_then(|e| e.description.clone()),
            element_automation_id: event.element.as_ref().and_then(|e| e.automation_id.clone()),
            element_bounds: event.element.as_ref().and_then(|e| {
                e.bounds.as_ref().map(|b| {
                    serde_json::json!({"x":b.x,"y":b.y,"width":b.width,"height":b.height})
                        .to_string()
                })
            }),
            frame_id: event.frame_id,
        };
        match event.data {
            EventData::Click {
                x,
                y,
                button,
                click_count,
                modifiers,
            } => {
                record.x = Some(x);
                record.y = Some(y);
                record.button = Some(button);
                record.click_count = Some(click_count);
                record.modifiers = Some(modifiers);
            }
            EventData::Move { x, y } => {
                record.x = Some(x);
                record.y = Some(y);
            }
            EventData::Scroll {
                x,
                y,
                delta_x,
                delta_y,
            } => {
                record.x = Some(x);
                record.y = Some(y);
                record.delta_x = Some(delta_x);
                record.delta_y = Some(delta_y);
            }
            EventData::Key {
                key_code,
                modifiers,
            } => {
                record.key_code = Some(key_code);
                record.modifiers = Some(modifiers);
            }
            EventData::Text { content, .. } => record.text_content = Some(content),
            EventData::AppSwitch { name, pid } => {
                record.app_name = Some(name.clone());
                record.text_content = Some(name);
                record.app_pid = Some(pid);
            }
            EventData::WindowFocus { app, title } => {
                record.app_name = Some(app.clone());
                record.window_title = title.clone();
                record.text_content = title.or(Some(app));
            }
            EventData::Clipboard { operation, content } => {
                record.modifiers = Some(operation as u8);
                record.text_content = content;
            }
        }
        record.text_content = clean(record.text_content);
        record.app_name = clean(record.app_name);
        record.window_title = clean(record.window_title);
        record.browser_url = clean(record.browser_url);
        record.element_role = clean(record.element_role);
        record.element_name = clean(record.element_name);
        record.element_value = clean(record.element_value);
        record.element_description = clean(record.element_description);
        record.element_automation_id = clean(record.element_automation_id);
        record
    }
}

pub async fn insert_ui_event_batch(
    pool: &SqlitePool,
    events: &[UiEventRecord],
) -> Result<Vec<i64>, sqlx::Error> {
    let mut tx = pool.begin().await?;
    let mut ids = Vec::with_capacity(events.len());
    for e in events {
        let result = sqlx::query("INSERT INTO ui_events (timestamp,session_id,relative_ms,event_type,x,y,delta_x,delta_y,button,click_count,key_code,modifiers,text_content,text_length,app_name,app_pid,window_title,browser_url,element_role,element_name,element_value,element_description,element_automation_id,element_bounds,frame_id) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18,?19,?20,?21,?22,?23,?24,?25)")
            .bind(e.timestamp.to_rfc3339()).bind(&e.session_id).bind(e.relative_ms)
            .bind(e.event_type).bind(e.x).bind(e.y).bind(e.delta_x).bind(e.delta_y)
            .bind(e.button).bind(e.click_count).bind(e.key_code).bind(e.modifiers)
            .bind(&e.text_content).bind(e.text_content.as_ref().map(|v| v.len() as i64))
            .bind(&e.app_name).bind(e.app_pid).bind(&e.window_title).bind(&e.browser_url)
            .bind(&e.element_role).bind(&e.element_name).bind(&e.element_value)
            .bind(&e.element_description).bind(&e.element_automation_id)
            .bind(&e.element_bounds).bind(e.frame_id).execute(&mut *tx).await?;
        ids.push(result.last_insert_rowid());
    }
    tx.commit().await?;
    Ok(ids)
}

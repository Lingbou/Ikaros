// SPDX-License-Identifier: GPL-3.0-only

use serde_json::{Value, json};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::browser) enum BrowserHttpAction {
    Status,
    List,
    New,
    Activate,
    Close,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::browser) enum BrowserWsAction {
    Navigate,
    Snapshot,
    Click,
    Type,
    Scroll,
    Screenshot,
    Cdp,
}

pub(in crate::browser) fn browser_input_schema(action: BrowserHttpAction) -> Value {
    let mut properties = json!({
        "endpoint": {
            "type": "string",
            "description": "Chrome DevTools HTTP endpoint. Defaults to http://127.0.0.1:9222."
        }
    });
    let mut required = Vec::<Value>::new();
    match action {
        BrowserHttpAction::New => {
            properties["url"] = json!({
                "type": "string",
                "description": "URL to open in the new target. Defaults to about:blank."
            });
        }
        BrowserHttpAction::Activate | BrowserHttpAction::Close => {
            properties["target_id"] = json!({
                "type": "string",
                "description": "CDP target id returned by browser_list or browser_new_target."
            });
            required.push(json!("target_id"));
        }
        BrowserHttpAction::Status | BrowserHttpAction::List => {}
    }
    json!({
        "type": "object",
        "required": required,
        "properties": properties,
    })
}

pub(in crate::browser) fn browser_ws_input_schema(action: BrowserWsAction) -> Value {
    let mut properties = json!({
        "endpoint": {
            "type": "string",
            "description": "Chrome DevTools HTTP endpoint. Defaults to http://127.0.0.1:9222."
        },
        "target_id": {
            "type": "string",
            "description": "CDP target id returned by browser_list or browser_new_target. A direct ws:// or wss:// debugger URL is also accepted."
        }
    });
    let mut required = vec![json!("target_id")];
    match action {
        BrowserWsAction::Navigate => {
            properties["url"] = json!({
                "type": "string",
                "description": "HTTP(S), file, or about:blank URL to navigate to."
            });
            required.push(json!("url"));
        }
        BrowserWsAction::Click => {
            properties["x"] = json!({"type": "number"});
            properties["y"] = json!({"type": "number"});
            required.push(json!("x"));
            required.push(json!("y"));
        }
        BrowserWsAction::Type => {
            properties["text"] = json!({"type": "string"});
            required.push(json!("text"));
        }
        BrowserWsAction::Scroll => {
            properties["x"] = json!({"type": "number", "default": 0});
            properties["y"] = json!({"type": "number", "default": 600});
        }
        BrowserWsAction::Screenshot => {
            properties["format"] = json!({
                "type": "string",
                "enum": ["png", "jpeg", "jpg", "webp"],
                "default": "png"
            });
            properties["output_path"] = json!({
                "type": "string",
                "description": "Optional workspace-relative path to save decoded screenshot bytes."
            });
        }
        BrowserWsAction::Cdp => {
            properties["method"] = json!({"type": "string"});
            properties["params"] = json!({"type": "object"});
            required.push(json!("method"));
        }
        BrowserWsAction::Snapshot => {}
    }
    json!({
        "type": "object",
        "required": required,
        "properties": properties,
    })
}

pub(in crate::browser) fn browser_action_is_write(action: BrowserHttpAction) -> bool {
    matches!(
        action,
        BrowserHttpAction::New | BrowserHttpAction::Activate | BrowserHttpAction::Close
    )
}

pub(in crate::browser) fn browser_action_name(action: BrowserHttpAction) -> &'static str {
    match action {
        BrowserHttpAction::Status => "status",
        BrowserHttpAction::List => "list",
        BrowserHttpAction::New => "new",
        BrowserHttpAction::Activate => "activate",
        BrowserHttpAction::Close => "close",
    }
}

pub(in crate::browser) fn browser_ws_action_is_write(action: BrowserWsAction) -> bool {
    matches!(
        action,
        BrowserWsAction::Navigate
            | BrowserWsAction::Click
            | BrowserWsAction::Type
            | BrowserWsAction::Scroll
            | BrowserWsAction::Cdp
    )
}

pub(in crate::browser) fn browser_ws_action_name(action: BrowserWsAction) -> &'static str {
    match action {
        BrowserWsAction::Navigate => "navigate",
        BrowserWsAction::Snapshot => "snapshot",
        BrowserWsAction::Click => "click",
        BrowserWsAction::Type => "type",
        BrowserWsAction::Scroll => "scroll",
        BrowserWsAction::Screenshot => "screenshot",
        BrowserWsAction::Cdp => "cdp",
    }
}

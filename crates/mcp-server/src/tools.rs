//! The tools an AI sees, and what each one maps to.
//!
//! # One table, no second path
//!
//! Every tool here names a method in the product API. There is no tool with a
//! handler of its own, so a tool cannot skip a permission check that the
//! desktop app performs, and a capability an agent can run is exactly the set
//! the interface can run.
//!
//! # The schemas are small on purpose
//!
//! A tool with fifteen optional parameters is one a model fills in wrongly.
//! Most of these take nothing, or take one string. The interesting complexity
//! is in what comes back.

use serde_json::{json, Value};

/// One tool, as the protocol describes it and as this crate routes it.
pub struct Tool {
    /// The name an agent calls.
    pub name: &'static str,
    /// What it does, written for a model deciding whether to call it.
    pub description: &'static str,
    /// The product API method it becomes.
    pub method: &'static str,
    /// Whether it only reads.
    pub read_only: bool,
}

impl Tool {
    /// The JSON Schema for this tool's arguments.
    pub fn schema(&self) -> Value {
        match self.name {
            "corescout_ask" => json!({
                "type": "object",
                "properties": {
                    "question": {
                        "type": "string",
                        "description": "What you want to know. For example: are there recurring \
                                        failures for this build? is there a verified way to \
                                        deploy? what state is the machine in?"
                    },
                    "operation": {
                        "type": "string",
                        "description": "Optional. The command or operation you are asking about."
                    },
                    "workspace": {
                        "type": "string",
                        "description": "Optional. The repository or folder you are working in."
                    }
                },
                "required": ["question"]
            }),
            "corescout_observe" => json!({
                "type": "object",
                "properties": {
                    "session": {
                        "type": "string",
                        "description": "A stable id for your session. Any string, the same one \
                                        each time."
                    },
                    "name": {
                        "type": "string",
                        "description": "The command line or tool name. Secrets are stripped \
                                        before anything is stored."
                    },
                    "kind": {
                        "type": "string",
                        "description": "Optional: shell, build, test, git, deploy, api, \
                                        file_edit, verification, tool_call."
                    },
                    "workspace": { "type": "string", "description": "Optional. The folder." },
                    "exit_code": { "type": "integer", "description": "Optional." },
                    "duration_ms": { "type": "integer", "description": "Optional." },
                    "reported": {
                        "type": "string",
                        "description": "What the tool said: success, failure, error, or silent. \
                                        Leave this out if nothing was said; do not guess."
                    },
                    "detail": { "type": "string", "description": "Optional failure text." },
                    "verified": {
                        "type": "string",
                        "description": "What you actually checked afterwards: confirmed, \
                                        contradicted, or unavailable. Leave it out if you did not \
                                        check. This field is the most valuable one here."
                    },
                    "verification_detail": {
                        "type": "string",
                        "description": "How reality disagreed, or why you could not check."
                    }
                },
                "required": ["session", "name"]
            }),
            "corescout_explain" => json!({
                "type": "object",
                "properties": {
                    "id": {
                        "type": "string",
                        "description": "The id of a learned item or capability."
                    }
                },
                "required": ["id"]
            }),
            "corescout_run_capability" => json!({
                "type": "object",
                "properties": {
                    "id": { "type": "string", "description": "The capability id." },
                    "dry_run": {
                        "type": "boolean",
                        "description": "Decide everything and start nothing. Use this first."
                    }
                },
                "required": ["id"]
            }),
            "corescout_recent_activity" => json!({
                "type": "object",
                "properties": {
                    "limit": { "type": "integer", "description": "How many entries. Default 30." },
                    "technical": {
                        "type": "boolean",
                        "description": "Include the low-level entries as well."
                    }
                }
            }),
            _ => json!({ "type": "object", "properties": {} }),
        }
    }

    /// Turn the protocol's arguments into the API's parameters.
    ///
    /// Almost always the same object. The exception is the activity tool,
    /// whose default the agent should not have to know.
    pub fn params(&self, arguments: &Value) -> Value {
        let mut params = if arguments.is_object() {
            arguments.clone()
        } else {
            json!({})
        };
        if self.name == "corescout_recent_activity" && params.get("limit").is_none() {
            params["limit"] = json!(30);
        }
        params
    }
}

/// Everything an agent may call.
pub const TOOLS: &[Tool] = &[
    Tool {
        name: "corescout_status",
        description: "Is CoreScout running, what is connected to it, and how much it has seen. \
                      Cheap; call it first if you are unsure whether CoreScout is available.",
        method: "status",
        read_only: true,
    },
    Tool {
        name: "corescout_briefing",
        description: "What CoreScout is, what it currently knows, and what it will and will not \
                      do in its present mode. Read this before relying on anything else here.",
        method: "briefing",
        read_only: true,
    },
    Tool {
        name: "corescout_knowledge",
        description: "Everything this computer has learned, grouped: about the AI tools that \
                      have worked here, about the repositories, about the machine, and what it \
                      is still uncertain about.",
        method: "knowledge",
        read_only: true,
    },
    Tool {
        name: "corescout_current_state",
        description: "What the machine is doing right now, and whether this is a way of running \
                      CoreScout recognises. An unfamiliar state means everything else CoreScout \
                      tells you is weaker than usual.",
        method: "live",
        read_only: true,
    },
    Tool {
        name: "corescout_machine",
        description: "What this computer physically is, and what CoreScout can observe of it.",
        method: "computer",
        read_only: true,
    },
    Tool {
        name: "corescout_learned",
        description: "Everything CoreScout has learned, as cards. Each one says whether it is a \
                      correlation it has merely observed or an effect it measured under \
                      randomised assignment. Check which before acting on one.",
        method: "learned",
        read_only: true,
    },
    Tool {
        name: "corescout_failures",
        description: "Operations that recur and go wrong on this machine, with how often and how \
                      they fail. Ask before attempting something that has a history here.",
        method: "failures",
        read_only: true,
    },
    Tool {
        name: "corescout_capabilities",
        description: "Procedures CoreScout has verified and the user has approved. Prefer one of \
                      these over doing the same thing by hand.",
        method: "capabilities",
        read_only: true,
    },
    Tool {
        name: "corescout_hypotheses",
        description: "What CoreScout is currently testing and cannot yet answer, and what \
                      evidence each is missing.",
        method: "hypotheses",
        read_only: true,
    },
    Tool {
        name: "corescout_ask",
        description: "Ask CoreScout what it knows about something: a repository, an operation, \
                      the machine's current state. Answers carry their evidence and their \
                      confidence.",
        method: "ask",
        read_only: true,
    },
    Tool {
        name: "corescout_explain",
        description: "Why CoreScout believes one thing: the evidence, whether it is randomised \
                      or merely observed, the alternative it considered, and what happened.",
        method: "explain",
        read_only: true,
    },
    Tool {
        name: "corescout_recent_activity",
        description: "What has happened recently, as CoreScout recorded it.",
        method: "activity",
        read_only: true,
    },
    Tool {
        name: "corescout_observe",
        description: "Tell CoreScout what you just did and how it went. The field that matters \
                      most is whether you actually verified the result: CoreScout learns from \
                      the gap between what a tool reports and what turns out to be true. Report \
                      failures and retries too; those are the useful ones.",
        method: "observe",
        read_only: false,
    },
    Tool {
        name: "corescout_run_capability",
        description: "Run a verified capability. Subject to the user's autonomy mode and \
                      permissions: it may be refused, or held for approval, and the reply says \
                      which. Try it with dry_run first.",
        method: "run",
        read_only: false,
    },
];

/// Find a tool by name.
pub fn find(name: &str) -> Option<&'static Tool> {
    TOOLS.iter().find(|tool| tool.name == name)
}

/// The tools as the protocol describes them.
pub fn describe() -> Vec<Value> {
    TOOLS
        .iter()
        .map(|tool| {
            json!({
                "name": tool.name,
                "description": tool.description,
                "inputSchema": tool.schema(),
                "annotations": {
                    "readOnlyHint": tool.read_only,
                    "openWorldHint": false,
                },
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_tool_has_a_name_a_description_and_a_method() {
        for tool in TOOLS {
            assert!(tool.name.starts_with("corescout_"), "{}", tool.name);
            assert!(
                tool.description.len() > 40,
                "{} needs a description a model can decide from",
                tool.name
            );
            assert!(!tool.method.is_empty());
        }
    }

    #[test]
    fn no_two_tools_share_a_name() {
        let mut names: Vec<&str> = TOOLS.iter().map(|tool| tool.name).collect();
        names.sort_unstable();
        let count = names.len();
        names.dedup();
        assert_eq!(names.len(), count);
    }

    #[test]
    fn every_tool_maps_to_a_method_the_api_actually_has() {
        // A tool naming a method that does not exist is one an agent can call
        // and always fail.
        let methods: Vec<&str> = corescout_product_api::api::METHODS
            .iter()
            .map(|(name, _)| *name)
            .collect();
        for tool in TOOLS {
            assert!(
                methods.contains(&tool.method),
                "{} maps to {:?}, which the API does not have",
                tool.name,
                tool.method
            );
        }
    }

    #[test]
    fn only_the_two_tools_that_change_anything_are_marked_as_such() {
        // The read-only hint is what a client uses to decide whether to ask
        // the user. Getting it wrong in the permissive direction is the one
        // that matters.
        let writing: Vec<&str> = TOOLS
            .iter()
            .filter(|tool| !tool.read_only)
            .map(|tool| tool.name)
            .collect();
        assert_eq!(
            writing,
            vec!["corescout_observe", "corescout_run_capability"]
        );
    }

    #[test]
    fn every_schema_is_a_json_schema_object() {
        for tool in TOOLS {
            let schema = tool.schema();
            assert_eq!(schema["type"], "object", "{}", tool.name);
            assert!(schema["properties"].is_object(), "{}", tool.name);
        }
    }

    #[test]
    fn the_reporting_tool_asks_for_verification_and_says_not_to_guess() {
        // The whole value of the reporting path is the honest verification
        // field. If the description does not push for it, agents will fill in
        // "success" from an exit code and CoreScout will learn nothing.
        let tool = find("corescout_observe").expect("the tool");
        let schema = tool.schema();
        let verified = schema["properties"]["verified"]["description"]
            .as_str()
            .expect("a description");
        assert!(verified.contains("did not check"), "{verified}");
        let reported = schema["properties"]["reported"]["description"]
            .as_str()
            .expect("a description");
        assert!(reported.contains("do not guess"), "{reported}");
    }

    #[test]
    fn the_activity_tool_supplies_a_default_the_agent_need_not_know() {
        let tool = find("corescout_recent_activity").expect("the tool");
        assert_eq!(tool.params(&json!({}))["limit"], 30);
        assert_eq!(tool.params(&json!({"limit": 5}))["limit"], 5);
    }

    #[test]
    fn arguments_that_are_not_an_object_do_not_become_one_by_accident() {
        let tool = find("corescout_status").expect("the tool");
        assert_eq!(tool.params(&json!(null)), json!({}));
        assert_eq!(tool.params(&json!("nonsense")), json!({}));
    }

    #[test]
    fn an_unknown_tool_is_not_found() {
        assert!(find("corescout_do_anything").is_none());
    }

    #[test]
    fn the_described_list_matches_the_table() {
        let described = describe();
        assert_eq!(described.len(), TOOLS.len());
        for entry in &described {
            assert!(entry["name"].is_string());
            assert!(entry["inputSchema"]["type"] == "object");
            assert!(entry["annotations"]["readOnlyHint"].is_boolean());
        }
    }
}

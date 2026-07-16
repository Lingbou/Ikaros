use crate::{
    domain::{Message, Role, ToolInvocation},
    provider::ModelProvider,
    store::SessionStore,
    tools::ToolBox,
};
use anyhow::{Result, bail};

const MAX_TOOL_ROUNDS_PER_TURN: usize = 8;

#[derive(Debug, Clone, PartialEq)]
pub enum RuntimeYield {
    Ready,
    Completed(String),
    AwaitingApproval(ToolInvocation),
    RecoveryRequired(ToolInvocation),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApprovalDecision {
    Approve,
    Deny,
}

pub struct Agent<P> {
    provider: P,
    store: SessionStore,
    tools: ToolBox,
}

impl<P> Agent<P>
where
    P: ModelProvider,
{
    pub fn new(provider: P, store: SessionStore, tools: ToolBox) -> Self {
        Self {
            provider,
            store,
            tools,
        }
    }

    pub fn store(&self) -> &SessionStore {
        &self.store
    }

    pub async fn resume(&self, session_id: &str) -> Result<RuntimeYield> {
        if let Some(invocation) = self.store.interrupted_tool(session_id)? {
            return Ok(RuntimeYield::RecoveryRequired(invocation));
        }
        if let Some(invocation) = self.store.pending_tool(session_id)? {
            return Ok(RuntimeYield::AwaitingApproval(invocation));
        }
        let messages = self.store.messages(session_id)?;
        match messages.last().map(|message| message.role) {
            Some(Role::User | Role::Tool) => self.continue_model(session_id).await,
            Some(Role::Assistant | Role::System) | None => Ok(RuntimeYield::Ready),
        }
    }

    pub async fn start_turn(&self, session_id: &str, user_text: &str) -> Result<RuntimeYield> {
        match self.resume(session_id).await? {
            RuntimeYield::Ready => {}
            pending => return Ok(pending),
        }
        let user_text = user_text.trim();
        if user_text.is_empty() {
            bail!("message is empty");
        }
        self.store
            .start_turn(session_id, &Message::user(user_text))?;
        self.continue_model(session_id).await
    }

    pub async fn decide(
        &self,
        session_id: &str,
        invocation_id: &str,
        decision: ApprovalDecision,
    ) -> Result<RuntimeYield> {
        match decision {
            ApprovalDecision::Approve => {
                let invocation = self.store.approve_tool(session_id, invocation_id)?;
                let result = self.tools.execute(&invocation.call).await;
                self.store.finish_tool(session_id, invocation_id, result)?;
            }
            ApprovalDecision::Deny => self.store.deny_tool(session_id, invocation_id)?,
        }
        if let Some(invocation) = self.store.pending_tool(session_id)? {
            return Ok(RuntimeYield::AwaitingApproval(invocation));
        }
        self.continue_model(session_id).await
    }

    pub async fn recover_interrupted(
        &self,
        session_id: &str,
        invocation_id: &str,
    ) -> Result<RuntimeYield> {
        self.store
            .mark_interrupted_failed(session_id, invocation_id)?;
        if let Some(invocation) = self.store.pending_tool(session_id)? {
            return Ok(RuntimeYield::AwaitingApproval(invocation));
        }
        self.continue_model(session_id).await
    }

    async fn continue_model(&self, session_id: &str) -> Result<RuntimeYield> {
        let stored_messages = self.store.messages(session_id)?;
        let tool_rounds = tool_rounds_since_last_user(&stored_messages);
        let mut messages = Vec::with_capacity(stored_messages.len() + 1);
        messages.push(Message::system(system_prompt(self.tools.workspace())));
        messages.extend(stored_messages);
        let reply = self
            .provider
            .complete(&messages, &self.tools.definitions())
            .await?;
        if tool_rounds >= MAX_TOOL_ROUNDS_PER_TURN && !reply.tool_calls.is_empty() {
            let content = format!(
                "Tool-call limit reached after {MAX_TOOL_ROUNDS_PER_TURN} rounds; no additional tool was executed."
            );
            self.store.record_assistant_reply(
                session_id,
                &Message::assistant(Some(content.clone()), Vec::new()),
            )?;
            return Ok(RuntimeYield::Completed(content));
        }
        let assistant = Message::assistant(reply.content.clone(), reply.tool_calls);
        self.store.record_assistant_reply(session_id, &assistant)?;
        if let Some(invocation) = self.store.pending_tool(session_id)? {
            return Ok(RuntimeYield::AwaitingApproval(invocation));
        }
        let content = assistant
            .content
            .ok_or_else(|| anyhow::anyhow!("provider returned an empty assistant reply"))?;
        Ok(RuntimeYield::Completed(content))
    }
}

fn system_prompt(workspace: &std::path::Path) -> String {
    format!(
        "You are Ikaros, a small local coding agent. The active workspace is {}. \
         Keep answers concise and use tools only when they materially help. Every tool call is \
         shown to the user for explicit approval. File tools are restricted to this workspace. \
         run_command starts a host process in the workspace and is not an OS sandbox. Use direct \
         program plus argument arrays; do not assume shell syntax is available.",
        workspace.display()
    )
}

fn tool_rounds_since_last_user(messages: &[Message]) -> usize {
    messages
        .iter()
        .rev()
        .take_while(|message| message.role != Role::User)
        .filter(|message| message.role == Role::Assistant && !message.tool_calls.is_empty())
        .count()
}

#[cfg(test)]
mod tests {
    use super::{Agent, ApprovalDecision, RuntimeYield};
    use crate::{
        domain::{AssistantReply, Message, Role, ToolCall},
        provider::ModelProvider,
        store::SessionStore,
        tools::ToolBox,
    };
    use anyhow::Result;
    use serde_json::{Value, json};
    use std::{collections::VecDeque, sync::Mutex};
    use tempfile::tempdir;

    struct ScriptedProvider {
        replies: Mutex<VecDeque<AssistantReply>>,
        requests: Mutex<Vec<Vec<Message>>>,
    }

    impl ScriptedProvider {
        fn new(replies: Vec<AssistantReply>) -> Self {
            Self {
                replies: Mutex::new(replies.into()),
                requests: Mutex::new(Vec::new()),
            }
        }
    }

    impl ModelProvider for ScriptedProvider {
        async fn complete(&self, messages: &[Message], _tools: &[Value]) -> Result<AssistantReply> {
            self.requests
                .lock()
                .map_err(|_| anyhow::anyhow!("requests lock poisoned"))?
                .push(messages.to_vec());
            self.replies
                .lock()
                .map_err(|_| anyhow::anyhow!("replies lock poisoned"))?
                .pop_front()
                .ok_or_else(|| anyhow::anyhow!("no scripted reply"))
        }
    }

    #[tokio::test]
    async fn approval_precedes_write_and_transcript_continues() {
        let temp = tempdir().expect("tempdir");
        let store = SessionStore::open(temp.path().join("sessions.sqlite3")).expect("store");
        let session = store
            .create_session(temp.path(), "test-model")
            .expect("session");
        let provider = ScriptedProvider::new(vec![
            AssistantReply {
                content: None,
                tool_calls: vec![ToolCall {
                    id: "call-write".to_owned(),
                    name: "write_file".to_owned(),
                    arguments: json!({"path": "result.txt", "content": "done"}),
                }],
            },
            AssistantReply {
                content: Some("Finished.".to_owned()),
                tool_calls: Vec::new(),
            },
        ]);
        let agent = Agent::new(
            provider,
            store.clone(),
            ToolBox::new(temp.path()).expect("tools"),
        );

        let first = agent
            .start_turn(&session.id, "write the result")
            .await
            .expect("start");
        let invocation_id = match first {
            RuntimeYield::AwaitingApproval(invocation) => invocation.id,
            other => panic!("expected approval, got {other:?}"),
        };
        assert!(!temp.path().join("result.txt").exists());

        let completed = agent
            .decide(&session.id, &invocation_id, ApprovalDecision::Approve)
            .await
            .expect("approve");
        assert_eq!(completed, RuntimeYield::Completed("Finished.".to_owned()));
        assert_eq!(
            std::fs::read_to_string(temp.path().join("result.txt")).expect("file"),
            "done"
        );
        let messages = store.messages(&session.id).expect("messages");
        assert_eq!(
            messages.last().map(|message| message.role),
            Some(Role::Assistant)
        );
        assert!(messages.iter().any(|message| message.role == Role::Tool));
    }

    #[tokio::test]
    async fn denial_has_no_side_effect_and_is_sent_back_to_model() {
        let temp = tempdir().expect("tempdir");
        let store = SessionStore::open(temp.path().join("sessions.sqlite3")).expect("store");
        let session = store
            .create_session(temp.path(), "test-model")
            .expect("session");
        let provider = ScriptedProvider::new(vec![
            AssistantReply {
                content: None,
                tool_calls: vec![ToolCall {
                    id: "call-write".to_owned(),
                    name: "write_file".to_owned(),
                    arguments: json!({"path": "denied.txt", "content": "no"}),
                }],
            },
            AssistantReply {
                content: Some("The write was denied.".to_owned()),
                tool_calls: Vec::new(),
            },
        ]);
        let agent = Agent::new(provider, store, ToolBox::new(temp.path()).expect("tools"));
        let pending = agent
            .start_turn(&session.id, "try a write")
            .await
            .expect("start");
        let invocation_id = match pending {
            RuntimeYield::AwaitingApproval(invocation) => invocation.id,
            other => panic!("expected approval, got {other:?}"),
        };
        let completed = agent
            .decide(&session.id, &invocation_id, ApprovalDecision::Deny)
            .await
            .expect("deny");
        assert_eq!(
            completed,
            RuntimeYield::Completed("The write was denied.".to_owned())
        );
        assert!(!temp.path().join("denied.txt").exists());
    }

    #[tokio::test]
    async fn interrupted_tools_are_not_replayed() {
        let temp = tempdir().expect("tempdir");
        let store = SessionStore::open(temp.path().join("sessions.sqlite3")).expect("store");
        let session = store
            .create_session(temp.path(), "test-model")
            .expect("session");
        store
            .record_assistant_reply(
                &session.id,
                &Message::assistant(
                    None,
                    vec![ToolCall {
                        id: "call-crash".to_owned(),
                        name: "write_file".to_owned(),
                        arguments: json!({"path": "unknown.txt", "content": "maybe"}),
                    }],
                ),
            )
            .expect("reply");
        let pending = store
            .pending_tool(&session.id)
            .expect("pending query")
            .expect("pending tool");
        store
            .approve_tool(&session.id, &pending.id)
            .expect("mark executing");
        let provider = ScriptedProvider::new(vec![AssistantReply {
            content: Some("Recovered without replay.".to_owned()),
            tool_calls: Vec::new(),
        }]);
        let agent = Agent::new(provider, store, ToolBox::new(temp.path()).expect("tools"));
        let state = agent.resume(&session.id).await.expect("resume");
        let recovery_id = match state {
            RuntimeYield::RecoveryRequired(invocation) => invocation.id,
            other => panic!("expected recovery, got {other:?}"),
        };
        assert!(!temp.path().join("unknown.txt").exists());
        let completed = agent
            .recover_interrupted(&session.id, &recovery_id)
            .await
            .expect("recover");
        assert_eq!(
            completed,
            RuntimeYield::Completed("Recovered without replay.".to_owned())
        );
    }

    #[tokio::test]
    async fn multiple_tools_are_approved_in_original_order() {
        let temp = tempdir().expect("tempdir");
        let store = SessionStore::open(temp.path().join("sessions.sqlite3")).expect("store");
        let session = store
            .create_session(temp.path(), "test-model")
            .expect("session");
        let provider = ScriptedProvider::new(vec![
            AssistantReply {
                content: None,
                tool_calls: vec![
                    ToolCall {
                        id: "call-one".to_owned(),
                        name: "write_file".to_owned(),
                        arguments: json!({"path": "one.txt", "content": "one"}),
                    },
                    ToolCall {
                        id: "call-two".to_owned(),
                        name: "write_file".to_owned(),
                        arguments: json!({"path": "two.txt", "content": "two"}),
                    },
                ],
            },
            AssistantReply {
                content: Some("Both finished.".to_owned()),
                tool_calls: Vec::new(),
            },
        ]);
        let agent = Agent::new(provider, store, ToolBox::new(temp.path()).expect("tools"));
        let first = agent
            .start_turn(&session.id, "write two files")
            .await
            .expect("start");
        let first_id = match first {
            RuntimeYield::AwaitingApproval(invocation) => {
                assert_eq!(invocation.call.id, "call-one");
                invocation.id
            }
            other => panic!("expected first approval, got {other:?}"),
        };
        let second = agent
            .decide(&session.id, &first_id, ApprovalDecision::Approve)
            .await
            .expect("first approval");
        let second_id = match second {
            RuntimeYield::AwaitingApproval(invocation) => {
                assert_eq!(invocation.call.id, "call-two");
                invocation.id
            }
            other => panic!("expected second approval, got {other:?}"),
        };
        let completed = agent
            .decide(&session.id, &second_id, ApprovalDecision::Approve)
            .await
            .expect("second approval");
        assert_eq!(
            completed,
            RuntimeYield::Completed("Both finished.".to_owned())
        );
        assert_eq!(
            std::fs::read_to_string(temp.path().join("one.txt")).expect("one"),
            "one"
        );
        assert_eq!(
            std::fs::read_to_string(temp.path().join("two.txt")).expect("two"),
            "two"
        );
    }

    #[tokio::test]
    async fn resume_finishes_a_turn_after_tool_result_was_persisted() {
        let temp = tempdir().expect("tempdir");
        let store = SessionStore::open(temp.path().join("sessions.sqlite3")).expect("store");
        let session = store
            .create_session(temp.path(), "test-model")
            .expect("session");
        store
            .start_turn(&session.id, &Message::user("read a file"))
            .expect("start turn");
        store
            .record_assistant_reply(
                &session.id,
                &Message::assistant(
                    None,
                    vec![ToolCall {
                        id: "call-read".to_owned(),
                        name: "read_file".to_owned(),
                        arguments: json!({"path": "README.md"}),
                    }],
                ),
            )
            .expect("assistant");
        let pending = store
            .pending_tool(&session.id)
            .expect("pending query")
            .expect("pending tool");
        store
            .approve_tool(&session.id, &pending.id)
            .expect("approve");
        store
            .finish_tool(&session.id, &pending.id, Ok("file contents".to_owned()))
            .expect("finish");
        let provider = ScriptedProvider::new(vec![AssistantReply {
            content: Some("Recovered final answer.".to_owned()),
            tool_calls: Vec::new(),
        }]);
        let agent = Agent::new(provider, store, ToolBox::new(temp.path()).expect("tools"));
        let outcome = agent.resume(&session.id).await.expect("resume");
        assert_eq!(
            outcome,
            RuntimeYield::Completed("Recovered final answer.".to_owned())
        );
    }

    #[tokio::test]
    async fn eighth_tool_round_still_allows_a_final_answer() {
        let temp = tempdir().expect("tempdir");
        let store = SessionStore::open(temp.path().join("sessions.sqlite3")).expect("store");
        let session = store
            .create_session(temp.path(), "test-model")
            .expect("session");
        store
            .start_turn(&session.id, &Message::user("complete eight tool rounds"))
            .expect("start turn");
        for index in 0..super::MAX_TOOL_ROUNDS_PER_TURN {
            store
                .record_assistant_reply(
                    &session.id,
                    &Message::assistant(
                        None,
                        vec![ToolCall {
                            id: format!("call-{index}"),
                            name: "read_file".to_owned(),
                            arguments: json!({"path": "README.md"}),
                        }],
                    ),
                )
                .expect("assistant");
            let pending = store
                .pending_tool(&session.id)
                .expect("pending query")
                .expect("pending tool");
            store
                .approve_tool(&session.id, &pending.id)
                .expect("approve");
            store
                .finish_tool(&session.id, &pending.id, Ok("contents".to_owned()))
                .expect("finish");
        }
        let provider = ScriptedProvider::new(vec![AssistantReply {
            content: Some("Final answer after eight rounds.".to_owned()),
            tool_calls: Vec::new(),
        }]);
        let agent = Agent::new(provider, store, ToolBox::new(temp.path()).expect("tools"));
        let outcome = agent.resume(&session.id).await.expect("resume");
        assert_eq!(
            outcome,
            RuntimeYield::Completed("Final answer after eight rounds.".to_owned())
        );
    }
}

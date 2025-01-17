//! a wrapper around rustyline
//!
//! (rustyline is a readline-like library for Rust)
//!
//! # ata²
//!
//!	 © 2023    Fredrick R. Brennan <copypaste@kittens.ph>
//!	 © 2023    Rik Huijzer <t.h.huijzer@rug.nl>
//!	 © 2023–   ATA Project Authors
//!
//!  Licensed under the Apache License, Version 2.0 (the "License");
//!  you may _not_ use this file except in compliance with the License.
//!  You may obtain a copy of the License at
//!
//!      http://www.apache.org/licenses/LICENSE-2.0
//!
//!  Unless required by applicable law or agreed to in writing, software
//!  distributed under the License is distributed on an "AS IS" BASIS,
//!  WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
//!  See the License for the specific language governing permissions and
//!  limitations under the License.

use async_openai::types::{
    ChatCompletionRequestAssistantMessage, ChatCompletionRequestMessage,
    ChatCompletionRequestUserMessage, ChatCompletionRequestUserMessageContent, Role,
};
use futures_util::lock::Mutex;
use rustyline::error::ReadlineError;
use rustyline::{
    Cmd, ConditionalEventHandler, Editor, EventContext, EventHandler, KeyCode, KeyEvent, Modifiers,
    RepeatCount,
};
use std::future::IntoFuture;
use std::io::Read as _;
use std::io::Write as _;
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::sync::mpsc::Sender;
use tokio::task::JoinHandle;

use std::sync::atomic::Ordering;
use std::sync::Arc;

use crate::prompt::{self, CONVERSATION};
use crate::TokioResult;
use crate::ABORT;
use crate::CONFIGURATION as config;
use crate::HAD_FIRST_INTERRUPT;

pub fn string_to_chat_completion_request_user_message(
    string: String,
) -> ChatCompletionRequestMessage {
    ChatCompletionRequestMessage::User(ChatCompletionRequestUserMessage {
        role: Role::User,
        content: Some(ChatCompletionRequestUserMessageContent::Text(string)),
        ..Default::default()
    })
}

pub fn string_to_chat_completion_assistant_message(string: String) -> ChatCompletionRequestMessage {
    ChatCompletionRequestMessage::Assistant(ChatCompletionRequestAssistantMessage {
        role: Role::Assistant,
        content: Some(string),
        ..Default::default()
    })
}

pub struct Readline {
    pub rl: Arc<Mutex<Editor<()>>>,
}

impl Readline {
    pub fn new() -> Self {
        let rl = Editor::<()>::new().unwrap();
        Self {
            rl: Arc::new(Mutex::new(rl)),
        }
    }
}

use futures_util::FutureExt as _;

struct RequestSaveHandler;
impl ConditionalEventHandler for RequestSaveHandler {
    fn handle(
        &self,
        _event: &rustyline::Event,
        _n: RepeatCount,
        _positive: bool,
        _: &EventContext,
    ) -> Option<Cmd> {
        let convo = CONVERSATION.lock().into_future();
        let convo = convo.now_or_never().unwrap();
        let convo = convo.clone();
        let convo_json = serde_json::to_string(&convo).unwrap();
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();
        // as unix secs
        let filename = format!("conversation-{}.json", now);
        let _ = std::fs::remove_file(&filename);
        let convo_file = std::fs::File::create(&filename).unwrap();
        let mut convo_file = std::io::BufWriter::new(convo_file);
        convo_file.write_all(convo_json.as_bytes()).unwrap();
        info!("Saved conversation to {filename}");
        Some(Cmd::Noop)
    }
}

impl Readline {
    pub async fn handle(&mut self, tx: Sender<Option<String>>) -> JoinHandle<TokioResult<()>> {
        let rl = self.rl.clone();
        let readline_handle: JoinHandle<TokioResult<()>> = tokio::spawn(async move {
            // If stdin is not a tty, we want to read once to the end of it and then exit.
            let mut already_read = false;
            let mut stdin = std::io::stdin();
            prompt::print_prompt();
            while !ABORT.load(Ordering::Relaxed) {
                // lock Readlien
                let mut rl = rl.lock().await;
                // Using an empty prompt text because otherwise the user would
                // "see" that the prompt is ready again during response printing.
                // Also, the current readline is cleared in some cases by rustyline,
                // so being on a newline is the only way to avoid that.
                let readline = if atty::is(atty::Stream::Stdin) {
                    rl.readline("")
                } else if !already_read {
                    let mut buf = String::with_capacity(1024);
                    stdin.read_to_string(&mut buf)?;
                    already_read = true;
                    Ok(buf)
                } else {
                    Err(ReadlineError::Eof)?
                };
                match readline {
                    Ok(line) => {
                        if line.is_empty() {
                            continue;
                        }
                        rl.add_history_entry(line.as_str());
                        tx.send(Some(line)).await?;
                        HAD_FIRST_INTERRUPT.store(false, Ordering::Relaxed);
                    }
                    Err(ReadlineError::Interrupted) => {
                        if config.ui.double_ctrlc && !HAD_FIRST_INTERRUPT.load(Ordering::Relaxed) {
                            HAD_FIRST_INTERRUPT.store(true, Ordering::Relaxed);
                            eprint!("\nPress Ctrl-C again to exit.");
                            prompt::print_prompt();
                            continue;
                        } else {
                            tx.send(None).await?;
                            ABORT.store(true, Ordering::Relaxed);
                            break;
                        }
                    }
                    Err(ReadlineError::Eof) => {
                        HAD_FIRST_INTERRUPT.store(false, Ordering::Relaxed);
                        tx.send(None).await?;
                        break;
                    }
                    Err(err) => {
                        eprintln!("{err:?}");
                        tx.send(None).await?;
                        break;
                    }
                }
            }
            return Ok(());
        });
        readline_handle
    }

    pub async fn enable_multiline(&mut self) {
        let mut rl = self.rl.lock().await;
        if config.ui.multiline_insertions {
            if atty::is(atty::Stream::Stdin) {
                // Cmd::Newline inserts a newline, Cmd::AcceptLine accepts the line
                rl.bind_sequence(KeyEvent(KeyCode::Enter, Modifiers::NONE), Cmd::Newline);
                rl.bind_sequence(
                    KeyEvent(KeyCode::Char('d'), Modifiers::CTRL),
                    Cmd::AcceptLine,
                );
            }
        }
    }

    pub async fn enable_request_save(&mut self) {
        let mut rl = self.rl.lock().await;
        if atty::is(atty::Stream::Stdin) {
            rl.bind_sequence(
                KeyEvent(KeyCode::F(2), Modifiers::NONE),
                EventHandler::Conditional(Box::new(RequestSaveHandler)),
            );
        }
    }

    pub async fn save_history(&mut self) -> TokioResult<()> {
        let mut rl = self.rl.lock().await;
        rl.save_history(&config.ui.history_file)?;
        Ok(())
    }

    pub async fn load_history(&mut self) -> TokioResult<()> {
        let mut rl = self.rl.lock().await;
        rl.load_history(&config.ui.history_file)?;
        Ok(())
    }

    pub async fn history_len(&mut self) -> usize {
        let rl = self.rl.lock().await;
        rl.history().len()
    }
}

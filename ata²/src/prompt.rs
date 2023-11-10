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

use ansi_colors::ColouredStr;
use async_openai::{
    config::OpenAIConfig,
    types::{
        ChatCompletionRequestUserMessageArgs, ChatCompletionResponseStreamMessage,
        CreateChatCompletionRequestArgs, FinishReason,
    },
    Client,
};
use atty;
use log::debug;

use futures_util::StreamExt as _;

use std::error::Error;
use std::io::Write;
use std::result::Result;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;
use std::sync::Arc;

pub type TokioResult<T, E = Box<dyn Error + Send + Sync>> = Result<T, E>;

fn print_and_flush(text: &str) {
    print!("{text}");
    std::io::stdout().flush().unwrap();
}

fn eprint_and_flush(text: &str) {
    eprint!("{text}");
    std::io::stderr().flush().unwrap();
}

pub fn eprint_bold(msg: &str) {
    if atty::is(atty::Stream::Stderr) {
        let mut bold = ColouredStr::new(msg);
        bold.bold();
        let bold = bold.to_string();
        eprint_and_flush(&bold.as_str());
    } else {
        eprint_and_flush(msg);
    }
}

pub fn print_prompt() {
    if atty::is(atty::Stream::Stderr) {
        eprint_bold("Prompt:\n");
    }
}

fn print_response_prompt() {
    if atty::is(atty::Stream::Stderr) {
        eprint_bold("Response:\n");
    }
}

fn finish_prompt(is_running: Arc<AtomicBool>) {
    is_running.store(false, Ordering::SeqCst);
    eprint_and_flush("\n\n");
    print_prompt();
}

pub fn print_error(is_running: Arc<AtomicBool>, msg: &str) {
    error!("{msg}");
    finish_prompt(is_running)
}

fn store_and_do_nothing(print_buffer: &mut Vec<String>, text: &str) -> String {
    print_buffer.push(text.to_string());
    "".to_string()
}

fn join_and_clear(print_buffer: &mut Vec<String>, text: &str) -> String {
    let from_buffer = print_buffer.join("");
    print_buffer.clear();
    let joined = format!("{from_buffer}{text}");
    joined.replace("\\n", "\n")
}

// Fixes cases where the model returns ["\", "n"] instead of ["\n"],
// which is interpreted as a newline in the OpenAI playground.
fn fix_newlines(print_buffer: &mut Vec<String>, text: &str) -> String {
    let single_backslash = r#"\"#;
    if text.ends_with(single_backslash) {
        return store_and_do_nothing(print_buffer, text);
    }
    if !print_buffer.is_empty() {
        return join_and_clear(print_buffer, text);
    }
    text.to_string()
}

fn post_process(print_buffer: &mut Vec<String>, text: &str) -> String {
    fix_newlines(print_buffer, text)
}

pub async fn request(
    abort: Arc<AtomicBool>,
    is_running: Arc<AtomicBool>,
    config: &super::Config,
    prompt: String,
    _count: i64,
) -> TokioResult<Vec<ChatCompletionResponseStreamMessage>> {
    let api_key: String = config.clone().api_key;
    let model: String = config.clone().model;
    let max_tokens: i64 = config.clone().max_tokens;
    let temperature: f64 = config.temperature;

    let mut print_buffer: Vec<String> = Vec::new();
    let oconfig = OpenAIConfig::new().with_api_key(api_key);
    let openai = Client::with_config(oconfig);
    let completions = openai.chat();
    let mut stream = completions
        .create_stream(
            CreateChatCompletionRequestArgs::default()
                .n(1)
                .messages(vec![ChatCompletionRequestUserMessageArgs::default()
                    .content(prompt)
                    .build()?
                    .into()])
                .model(&model)
                .max_tokens(max_tokens as u16)
                .temperature(temperature as f32)
                .stream(true)
                .build()?,
        )
        .await?;
    is_running.store(true, Ordering::SeqCst);

    let got_first_success: Arc<AtomicBool> = Arc::new(AtomicBool::new(false));
    let mut ret = vec![];

    'abort: while !abort.load(Ordering::Relaxed) {
        'outer: while let Some(completion) = stream.next().await {
            debug!("Completion: {:?}", completion);
            match completion {
                Ok(completion) => {
                    let completion = Arc::new(completion);
                    ret.push(completion.clone());
                    if !got_first_success.load(Ordering::SeqCst) {
                        got_first_success.store(true, Ordering::SeqCst);
                        print_response_prompt();
                    }
                    for choice in &completion.choices {
                        if abort.load(Ordering::Relaxed) {
                            break 'abort;
                        }
                        match choice.finish_reason {
                            Some(FinishReason::Stop) => {
                                debug!("Got stop from API, returning to REPL");
                                break 'abort;
                            }
                            Some(reason) => {
                                let msg = format!("OpenAI API error: {reason:?}");
                                print_error(is_running.clone(), &msg);
                                continue 'abort;
                            }
                            None => {}
                        }
                        match choice.delta.content {
                            Some(ref text) => {
                                let newline_fixed = post_process(&mut print_buffer, &text);
                                print_and_flush(&newline_fixed);
                            }
                            None => {
                                continue 'outer;
                            }
                        }
                    }
                }
                Err(e) => {
                    let msg = format!("OpenAI API error: {e}");
                    print_error(is_running.clone(), &msg);
                    continue 'abort;
                }
            }
        }
        break 'abort;
    }

    if !got_first_success.load(Ordering::SeqCst) {
        let msg = format!("Empty prompt, aborting.");
        print_error(is_running.clone(), &msg);
        return Ok(vec![]);
    }

    print_and_flush("\n");
    let result = ret
        .drain(..)
        .map(|o| Arc::new(o.choices.clone().into_iter().collect::<Vec<_>>()))
        .collect::<Vec<_>>()
        .drain(..)
        .map(|choice: Arc<Vec<ChatCompletionResponseStreamMessage>>| {
            choice
                .iter()
                .map(|choice| choice.clone())
                .collect::<Vec<_>>()
        })
        .flatten()
        .collect::<Vec<_>>();

    is_running.store(false, Ordering::SeqCst);
    finish_prompt(is_running);
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn leading_newlines() {
        assert_eq!(
            sanitize_input("foo\"bar".to_string()),
            "foo\\\"bar".to_string()
        );
    }
}

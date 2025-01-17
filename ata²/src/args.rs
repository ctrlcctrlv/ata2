//! Command-line argument parsing (using [`clap`]).
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

use crate::config::ConfigLocation;

use clap::Parser;
use clap::{crate_authors, crate_version};

#[derive(Parser, Debug)]
#[command(author = crate_authors!(), version = crate_version!(),
    about, long_about = None,
    help_template = "{before-help}{name} {version} — {about}\
    \n\n\
    © 2023\t{author}\
    \n\n\
    {usage-heading} {usage}\
    \n\n\
    {all-args}{after-help}")]
pub struct Ata2 {
    /// Path to the configuration TOML file.
    #[arg(short = 'c', long = "config", default_value = "")]
    pub config: ConfigLocation,

    /// Avoid printing the configuration to stdout.
    #[arg(long)]
    pub hide_config: bool,

    /// Print the keyboard shortcuts.
    #[arg(long)]
    pub print_shortcuts: bool,

    /// Conversation file to load.
    #[arg(short = 'l', long = "load")]
    pub load: Option<String>,
}

// Deny usage of print and eprint as it won't have same result
// in WASI as if doing in standard program, you must really know
// what you are doing to disable that lint (and you don't know)
#![deny(clippy::print_stdout)]
#![deny(clippy::print_stderr)]

use anyhow::Result;
use github_release_check::{self, GitHub};
use lapce_plugin::{
    psp_types::{
        lsp_types::{
            request::Initialize, Command, DocumentFilter, DocumentSelector, InitializeParams,
            MessageType, Url,
        },
        Request,
    },
    register_plugin, Http, LapcePlugin, VoltEnvironment, PLUGIN_RPC,
};
use regex::Regex::new as Regexp;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::env::var as env_var;
use std::process::Command;

#[derive(Default)]
struct State {}

register_plugin!(State);

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginInfo {
    java_version: String,
    scala_version: String,
    // project and system sbt version
    sbt_version: Vec<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Configuration {
    language_id: String,
    options: Option<Value>,
}

register_plugin!(State);

fn initialize(params: InitializeParams) -> Result<()> {
    let server_path = params
        .initialization_options
        .as_ref()
        .and_then(|options| options.get("serverPath"))
        .and_then(|server_path| server_path.as_str())
        .and_then(|server_path| {
            if server_path.is_empty() {
                None
            } else {
                Some(server_path)
            }
        });

    if let Some(server_path) = server_path {
        PLUGIN_RPC.start_lsp(
            rl::parse(&format!("urn:{}", server_path))?,
            vec![],
            vec![DocumentFilter {
                language: Some("scala".to_string()),
                scheme: None,
                pattern: Some(
                    "**/*.
                {scala}"
                        .to_string(),
                ),
            }],
            params.initialization_options,
        );
        return Ok(());
    }

    let java_version = Command::new(
        "java_version".to_string(),
        "java".to_string(),
        "-version".to_string(),
    )
    .output()
    .map(|output| {
        String::from_utf8_lossy(&output.stderr)
            .lines()
            .next()
            .unwrap_or_default()
            .to_string()
    })
    .unwrap_or_default();

    // for scala we're actually only interested in the build tag,
    // primarilfy due to Scala 2 and Scala 3 differences
    // Therefore we'll trim the output of this command based on regex \d+\.\d+\.\d+
    let scala_version = Some(
        Command::new(
            "scala_version".to_string(),
            "scala".to_string(),
            "-version".to_string(),
        )
        .output()
        .map(|output| {
            let output = String::from_utf8_lossy(&output.stderr)
                .lines()
                .next()
                .unwrap_or_default()
                .to_string();
            let re = Regexp(r"\d+\.\d+\.\d+").unwrap();
            re.find(&output)
                .map(|m| m.as_str().to_string())
                .unwrap_or_default()
        })
        .unwrap_or_default(),
    );

    // for sbt we'll build a vec of versions for system and project, using the pollowing format:
    // system: $tag, project: $tag
    // so that
    // sbt version in this project: 1.9.9
    // sbt script version: 1.9.9
    // would result in ["1.9.9", "1.9.9"]
    let sbt_version = Command::new(
        "sbt_version".to_string(),
        "sbt".to_string(),
        "-version".to_string(),
    )
    .output()
    .map(|output| {
        let output = String::from_utf8_lossy(&output.stderr)
            .lines()
            .next()
            .unwrap_or_default()
            .to_string();
        let re = Regexp(r"\d+\.\d+\.\d+").unwrap();
        re.find_iter(&output)
            .map(|m| m.as_str().to_string())
            .collect::<Vec<String>>()
    });

    let document_selector: DocumentSelector = vec![DocumentFilter {
        // lsp language id
        language: Some(String::from("scala")),
        // glob pattern
        pattern: Some(String::from("**/*.{scala}")),
        // like file:
        scheme: None,
    }];
    let mut server_args = vec![];
    let mut options = None;

    // Check for user specified LSP server path
    // ```
    // [lapce-plugin-name.lsp]
    // serverPath = "[path or filename]"
    // serverArgs = ["--arg1", "--arg2"]
    // ```
    if let Some(options) = params.initialization_options.as_ref() {
        if let Some(lsp) = options.get("lsp") {
            if let Some(args) = lsp.get("serverArgs") {
                if let Some(args) = args.as_array() {
                    if !args.is_empty() {
                        server_args = vec![];
                    }
                    for arg in args {
                        if let Some(arg) = arg.as_str() {
                            server_args.push(arg.to_string());
                        }
                    }
                }
            }

            if let Some(server_path) = lsp.get("serverPath") {
                if let Some(server_path) = server_path.as_str() {
                    if !server_path.is_empty() {
                        let server_uri = Url::parse(&format!("urn:{}", server_path))?;
                        PLUGIN_RPC.start_lsp(
                            server_uri,
                            server_args,
                            document_selector,
                            params.initialization_options,
                        );
                        return Ok(());
                    }
                }
            }
        }
    }

    // Download URL
    // let _ = format!("https://github.com/<name>/<project>/releases/download/<version>/{filename}");

    // see lapce_plugin::Http for available API to download files

    let latest_jdk_release = get_latest_release_for("adoptium/temurin21-binaries")?;

    let jdk_url = match VoltEnvironment::operating_system().as_deref() {
        Ok("windows") => {
            format!("{}.exe", "https://github.com/adoptium/temurin21-binaries/releases/download/jdk-21.0.2%2B13/OpenJDK21U-jdk_x64_windows_hotspot_21.0.2_13.msi")
        }
        _ => "[filename]".to_string(),
    };

    // Plugin working directory
    let volt_uri = VoltEnvironment::uri()?;
    let server_uri = Url::parse(&volt_uri)?.join("[filename]")?;

    // if you want to use server from PATH
    // let server_uri = Url::parse(&format!("urn:{filename}"))?;

    // Available language IDs
    // https://github.com/lapce/lapce/blob/HEAD/lapce-proxy/src/buffer.rs#L173
    PLUGIN_RPC.start_lsp(
        server_uri,
        server_args,
        document_selector,
        params.initialization_options,
    );

    Ok(())
}

// WARN: might need to get e.g. latest 100 releases and then filter for the latest stable one
fn get_latest_release_for(repo: &str) -> Result<String> {
    let github = GitHub::new().unwrap();
    let latest_version = github.get_latest_version(repo)?;
    latest_version
}

#[test]
fn test_get_latest_release_for() {
    // luckily for us the crate for this hasn't been updated in >1yr since commiting this
    // so is a good test object
    let repo = "celeo/github_release_check";
    let latest_version = get_latest_release_for(repo).unwrap();
    assert_eq!(latest_version, "0.2.1");
}

// extract major version from release
// e.g. forr 21.0.2+13, get OpenJDK21U
fn read_major_jdk_version(release: &str) -> String {
    let re = Regexp(r"(\d+)\.(\d+)\.(\d+)\+\d+").unwrap();
    let captures = re.captures(release).unwrap();
    format!("OpenJDK{}U", captures.get(1).unwrap().as_str())
}

#[test]
fn test_read_major_jdk_version() {
    let release = "21.0.2+13";
    let major_version = read_major_jdk_version(release);
    assert_eq!(major_version, "OpenJDK21U");
}

impl LapcePlugin for State {
    fn handle_request(&mut self, _id: u64, method: String, params: Value) {
        #[allow(clippy::single_match)]
        match method.as_str() {
            Initialize::METHOD => {
                let params: InitializeParams = serde_json::from_value(params).unwrap();
                if let Err(e) = initialize(params) {
                    PLUGIN_RPC.window_show_message(
                        MessageType::ERROR,
                        format!("plugin returned with error: {e}"),
                    )
                }
            }
            _ => {}
        }
    }
}

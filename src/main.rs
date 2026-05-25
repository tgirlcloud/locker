use argh::FromArgs;
use serde::Deserialize;
use std::error::Error;
use std::fs;
use std::{collections::HashMap, path::PathBuf};

/// locker - a tool to lint your flake.lock file
#[derive(FromArgs)]
#[argh(help_triggers("-h", "--help"))]
struct Args {
    #[argh(positional, default = "PathBuf::from(\"flake.lock\")")]
    flake_lock: PathBuf,
}

#[derive(Deserialize, Debug)]
struct FlakeLock {
    nodes: HashMap<String, Node>,
    version: usize,

    #[allow(dead_code)]
    root: String,
}

#[derive(Deserialize, Debug)]
#[serde(untagged)]
#[allow(dead_code)]
enum InputRef {
    Node(String),
    Path(Vec<String>),
}

#[derive(Deserialize, Debug)]
struct Node {
    locked: Option<Locked>,
    inputs: Option<HashMap<String, InputRef>>,
}

/// https://nix.dev/manual/nix/2.34/command-ref/new-cli/nix3-flake.html#types
#[derive(Deserialize, Debug, Eq, PartialEq, Clone)]
#[serde(tag = "type", rename_all = "lowercase")]
enum Locked {
    // scm
    GitHub { owner: String, repo: String },
    GitLab { owner: String, repo: String },
    SourceHut { owner: String, repo: String },

    // url
    Git { url: String },
    Hg { url: String },
    Tarball { url: String },
    File { url: String },

    // path
    Path { path: String },
}

fn main() -> Result<(), Box<dyn Error>> {
    let args: Args = argh::from_env();
    let flake_lock_content = fs::read_to_string(&args.flake_lock)?;
    let flake_lock: FlakeLock = serde_json::from_str(&flake_lock_content)?;

    if flake_lock.version != 7 {
        eprintln!("Unsupported flake.lock version: {}", flake_lock.version);
        std::process::exit(1);
    }

    let inputs = parse_inputs(&flake_lock);
    let duplicates = find_duplicates(&flake_lock, inputs);

    if duplicates.is_empty() {
        println!("No duplicate inputs found.");
        std::process::exit(0);
    }

    println!("The following flake uris contained duplicate entries in your flake.lock:");
    for (input, dups) in duplicates {
        eprintln!("  '{}': {}", input, dups.join(", "));
    }

    std::process::exit(1);
}

fn parse_inputs(flake_lock: &FlakeLock) -> HashMap<String, String> {
    let mut data = HashMap::new();

    for (k, v) in &flake_lock.nodes {
        if let Some(locked) = &v.locked {
            let val = flake_uri(locked.clone());
            data.insert(k.clone(), val);
        }
    }

    data
}

fn find_paths_to_nodes(flake_lock: &FlakeLock) -> HashMap<String, Vec<String>> {
    let mut node_paths: HashMap<String, Vec<String>> = HashMap::new();
    let mut visited = std::collections::HashSet::new();

    let mut queue = std::collections::VecDeque::new();
    queue.push_back((flake_lock.root.clone(), String::new()));

    while let Some((node_id, current_path)) = queue.pop_front() {
        if !current_path.is_empty() {
            node_paths
                .entry(node_id.clone())
                .or_default()
                .push(current_path.clone());
        }

        if visited.contains(&node_id) {
            continue;
        }
        visited.insert(node_id.clone());

        if let Some(node) = flake_lock.nodes.get(&node_id) {
            if let Some(inputs) = &node.inputs {
                let mut sorted_inputs: Vec<_> = inputs.iter().collect();
                sorted_inputs.sort_by_key(|k| k.0);

                for (input_name, input_ref) in sorted_inputs {
                    if let InputRef::Node(target_id) = input_ref {
                        let next_path = if current_path.is_empty() {
                            format!("inputs.{}", input_name)
                        } else {
                            format!("{}.inputs.{}", current_path, input_name)
                        };
                        queue.push_back((target_id.clone(), next_path));
                    }
                }
            }
        }
    }

    node_paths
}

fn find_duplicates(
    flake_lock: &FlakeLock,
    inputs: HashMap<String, String>,
) -> HashMap<String, Vec<String>> {
    let mut counts: HashMap<String, Vec<String>> = HashMap::new();
    let paths = find_paths_to_nodes(flake_lock);

    for (node_id, input_uri) in inputs {
        if let Some(node_paths) = paths.get(&node_id) {
            for path in node_paths {
                counts
                    .entry(input_uri.clone())
                    .or_default()
                    .push(format!("{} ({})", node_id, path));
            }
        } else if node_id != flake_lock.root {
            counts.entry(input_uri).or_default().push(node_id);
        }
    }

    counts.into_iter().filter(|(_, v)| v.len() > 1).collect()
}

fn flake_uri(lock: Locked) -> String {
    match lock {
        Locked::GitHub { owner, repo } => make_scm_uri("github", &owner, &repo),
        Locked::GitLab { owner, repo } => make_scm_uri("gitlab", &owner, &repo),
        Locked::SourceHut { owner, repo } => make_scm_uri("sourcehut", &owner, &repo),
        Locked::Git { url } => make_url_uri("git", &url),
        Locked::Hg { url } => make_url_uri("hg", &url),
        Locked::Tarball { url } => make_url_uri("tarball", &url),
        Locked::File { url} => make_url_uri("file", &url),
        Locked::Path { path } => format!("path:{path}"),
    }
}

fn make_scm_uri(node_type: &str, owner: &str, repo: &str) -> String {
    format!(
        "{node_type}:{}/{}",
        owner.to_lowercase(),
        repo.to_lowercase()
    )
}

fn make_url_uri(node_type: &str, url: &str) -> String {
    format!("{node_type}:{url}")
}

#[cfg(test)]
mod tests {
    use super::*;

    const FLAKE_LOCK: &str = r#"
    {
        "nodes": {
            "input1": {
                "locked": {
                    "type": "github",
                    "owner": "user1",
                    "repo": "repo1"
                }
            },
            "input2": {
                "locked": {
                    "type": "github",
                    "owner": "user2",
                    "repo": "repo2"
                }
            },
            "input3": {
                "locked": {
                    "type": "github",
                    "owner": "user1",
                    "repo": "repo1"
                }
            },
            "input4": {
                "locked": {
                    "type": "git",
                    "url": "https://example.com/repo.git"
                }
            },
            "input5": {
                "locked": {
                    "type": "git",
                    "url": "https://example.com/repo.git"
                }
            }
        },
        "version": 7,
        "root": "."
    }
    "#;

    #[test]
    fn test_parse_inputs() {
        let flake_lock: FlakeLock = serde_json::from_str(FLAKE_LOCK).unwrap();
        let inputs = parse_inputs(&flake_lock);

        assert_eq!(inputs.len(), 5);
        assert!(inputs.contains_key("input1"));
        assert!(inputs.contains_key("input2"));
        assert!(inputs.contains_key("input3"));
        assert!(inputs.contains_key("input4"));
        assert!(inputs.contains_key("input5"));

        assert_eq!(inputs.get("input1").unwrap(), "github:user1/repo1");
        assert_eq!(inputs.get("input2").unwrap(), "github:user2/repo2");
        assert_eq!(inputs.get("input3").unwrap(), "github:user1/repo1");
        assert_eq!(
            inputs.get("input4").unwrap(),
            "git:https://example.com/repo.git"
        );
        assert_eq!(
            inputs.get("input5").unwrap(),
            "git:https://example.com/repo.git"
        );
    }

    #[test]
    fn test_duplicates() {
        let flake_lock: FlakeLock = serde_json::from_str(FLAKE_LOCK).unwrap();

        let inputs = parse_inputs(&flake_lock);
        let duplicates = find_duplicates(&flake_lock, inputs.clone());

        assert_eq!(duplicates.len(), 2);
    }

    #[test]
    fn test_duplicates_2() -> Result<(), Box<dyn Error>> {
        let flake_lock_contents = fs::read_to_string("test/flake-lock.json")?;
        let flake_lock: FlakeLock = serde_json::from_str(&flake_lock_contents)?;

        let inputs = parse_inputs(&flake_lock);
        let duplicates = find_duplicates(&flake_lock, inputs);

        assert_eq!(duplicates.len(), 13);
        assert!(duplicates.contains_key("github:nixos/nixpkgs"));
        assert_eq!(duplicates.get("github:nixos/nixpkgs").unwrap().len(), 7);

        assert_eq!(
            duplicates
                .get("tarball:https://api.flakehub.com/f/pinned/edolstra/flake-compat/1.0.1/018afb31-abd1-7bff-a5e4-cff7e18efb7a/source.tar.gz")
                .unwrap()
                .len(),
            2
        );

        Ok(())
    }
}

use argh::FromArgs;
use serde::Deserialize;
use std::error::Error;
use std::fs;
use std::{
    collections::{HashMap, HashSet},
    path::PathBuf,
};

/// locker - a tool to lint your flake.lock file
#[derive(FromArgs)]
#[argh(help_triggers("-h", "--help"))]
struct Args {
    #[argh(positional, default = "PathBuf::from(\"flake.lock\")")]
    flake_lock: PathBuf,

    /// input name to ignore when checking for duplicates (may be repeated)
    #[argh(option, short = 'i')]
    ignore: Vec<String>,

    /// treat inputs that track different branches (refs) as distinct rather
    /// than duplicates (off by default)
    #[argh(switch, short = 'b')]
    distinct_branches: bool,
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
    original: Option<Original>,
}

/// The `original` flake reference, before locking. The branch a user tracks
/// lives here (e.g. `nixos-unstable`), not in `locked`.
#[derive(Deserialize, Debug)]
struct Original {
    #[serde(rename = "ref")]
    git_ref: Option<String>,
}

/// https://nix.dev/manual/nix/2.34/command-ref/new-cli/nix3-flake.html#types
#[derive(Deserialize, Debug)]
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

    let ignore: HashSet<String> = args.ignore.into_iter().collect();
    let inputs = parse_inputs(&flake_lock, args.distinct_branches);
    let duplicates = find_duplicates(&flake_lock, inputs, &ignore);

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

fn parse_inputs(flake_lock: &FlakeLock, distinct_branches: bool) -> HashMap<String, String> {
    let mut data = HashMap::new();

    for (k, v) in &flake_lock.nodes {
        if let Some(locked) = &v.locked {
            let mut uri = flake_uri(locked);
            if distinct_branches
                && let Some(git_ref) = v.original.as_ref().and_then(|o| o.git_ref.as_deref())
            {
                uri.push('/');
                uri.push_str(git_ref);
            }
            data.insert(k.clone(), uri);
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

        if let Some(node) = flake_lock.nodes.get(&node_id) && let Some(inputs) = &node.inputs {
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

    node_paths
}

fn find_duplicates(
    flake_lock: &FlakeLock,
    inputs: HashMap<String, String>,
    ignore: &HashSet<String>,
) -> HashMap<String, Vec<String>> {
    let mut counts: HashMap<String, Vec<String>> = HashMap::new();
    let paths = find_paths_to_nodes(flake_lock);

    for (node_id, input_uri) in inputs {
        if ignore.contains(&node_id) {
            continue;
        }

        if let Some(node_paths) = paths.get(&node_id) {
            let entry = counts.entry(input_uri).or_default();
            for path in node_paths {
                entry.push(format!("{} ({})", node_id, path));
            }
        } else if node_id != flake_lock.root {
            counts.entry(input_uri).or_default().push(node_id);
        }
    }

    counts.into_iter().filter(|(_, v)| v.len() > 1).collect()
}

fn flake_uri(lock: &Locked) -> String {
    match lock {
        Locked::GitHub { owner, repo } => make_scm_uri("github", owner, repo),
        Locked::GitLab { owner, repo } => make_scm_uri("gitlab", owner, repo),
        Locked::SourceHut { owner, repo } => make_scm_uri("sourcehut", owner, repo),
        Locked::Git { url } => make_url_uri("git", url),
        Locked::Hg { url } => make_url_uri("hg", url),
        Locked::Tarball { url } => make_url_uri("tarball", url),
        Locked::File { url } => make_url_uri("file", url),
        Locked::Path { path } => format!("path:{path}"),
    }
}

fn make_scm_uri(node_type: &str, owner: &str, repo: &str) -> String {
    let mut s = String::with_capacity(node_type.len() + owner.len() + repo.len() + 2);
    s.push_str(node_type);
    s.push(':');
    s.extend(owner.chars().flat_map(char::to_lowercase));
    s.push('/');
    s.extend(repo.chars().flat_map(char::to_lowercase));
    s
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
        let inputs = parse_inputs(&flake_lock, false);

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

        let inputs = parse_inputs(&flake_lock, false);
        let duplicates = find_duplicates(&flake_lock, inputs.clone(), &HashSet::new());

        assert_eq!(duplicates.len(), 2);
    }

    #[test]
    fn test_ignore_excludes_named_inputs() {
        let flake_lock: FlakeLock = serde_json::from_str(FLAKE_LOCK).unwrap();
        let inputs = parse_inputs(&flake_lock, false);

        // input1 and input3 both resolve to github:user1/repo1. Ignoring
        // input3 leaves a single occurrence, so it is no longer a duplicate.
        let ignore: HashSet<String> = ["input3".to_string()].into_iter().collect();
        let duplicates = find_duplicates(&flake_lock, inputs, &ignore);

        assert_eq!(duplicates.len(), 1);
        assert!(!duplicates.contains_key("github:user1/repo1"));
        assert!(duplicates.contains_key("git:https://example.com/repo.git"));
    }

    // Two inputs tracking the same repo on different branches.
    const BRANCHED_LOCK: &str = r#"
    {
        "nodes": {
            "nixpkgs-unstable": {
                "locked": { "type": "github", "owner": "nixos", "repo": "nixpkgs", "rev": "aaa" },
                "original": { "type": "github", "owner": "nixos", "repo": "nixpkgs", "ref": "nixos-unstable" }
            },
            "nixpkgs-stable": {
                "locked": { "type": "github", "owner": "nixos", "repo": "nixpkgs", "rev": "bbb" },
                "original": { "type": "github", "owner": "nixos", "repo": "nixpkgs", "ref": "nixos-24.05" }
            },
            "root": {
                "inputs": {
                    "nixpkgs-unstable": "nixpkgs-unstable",
                    "nixpkgs-stable": "nixpkgs-stable"
                }
            }
        },
        "version": 7,
        "root": "root"
    }
    "#;

    #[test]
    fn test_branches_collapsed_by_default() {
        // Without --distinct-branches, differing branches share a uri and are
        // reported as a duplicate.
        let flake_lock: FlakeLock = serde_json::from_str(BRANCHED_LOCK).unwrap();
        let inputs = parse_inputs(&flake_lock, false);
        let duplicates = find_duplicates(&flake_lock, inputs, &HashSet::new());

        assert_eq!(duplicates.len(), 1);
        assert!(duplicates.contains_key("github:nixos/nixpkgs"));
    }

    #[test]
    fn test_distinct_branches_separates_refs() {
        // With distinct_branches, the ref is folded into the uri so the two
        // nixpkgs inputs are no longer duplicates.
        let flake_lock: FlakeLock = serde_json::from_str(BRANCHED_LOCK).unwrap();
        let inputs = parse_inputs(&flake_lock, true);

        assert_eq!(
            inputs.get("nixpkgs-unstable").unwrap(),
            "github:nixos/nixpkgs/nixos-unstable"
        );
        assert_eq!(
            inputs.get("nixpkgs-stable").unwrap(),
            "github:nixos/nixpkgs/nixos-24.05"
        );

        let duplicates = find_duplicates(&flake_lock, inputs, &HashSet::new());
        assert!(duplicates.is_empty());
    }

    #[test]
    fn test_distinct_branches_keeps_same_branch_duplicates() {
        // Same repo, same (absent) branch: still a duplicate even with the
        // flag on, because nothing distinguishes them.
        let lock = r#"
        {
            "nodes": {
                "a": {
                    "locked": { "type": "github", "owner": "nixos", "repo": "nixpkgs" },
                    "original": { "type": "github", "owner": "nixos", "repo": "nixpkgs" }
                },
                "b": {
                    "locked": { "type": "github", "owner": "nixos", "repo": "nixpkgs" },
                    "original": { "type": "github", "owner": "nixos", "repo": "nixpkgs" }
                },
                "root": { "inputs": { "a": "a", "b": "b" } }
            },
            "version": 7,
            "root": "root"
        }
        "#;

        let flake_lock: FlakeLock = serde_json::from_str(lock).unwrap();
        let inputs = parse_inputs(&flake_lock, true);
        let duplicates = find_duplicates(&flake_lock, inputs, &HashSet::new());

        assert_eq!(duplicates.len(), 1);
        assert!(duplicates.contains_key("github:nixos/nixpkgs"));
    }

    #[test]
    fn test_duplicates_2() -> Result<(), Box<dyn Error>> {
        let flake_lock_contents = fs::read_to_string("test/flake-lock.json")?;
        let flake_lock: FlakeLock = serde_json::from_str(&flake_lock_contents)?;

        let inputs = parse_inputs(&flake_lock, false);
        let duplicates = find_duplicates(&flake_lock, inputs, &HashSet::new());

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

    #[test]
    fn test_flake_uri_scm_variants() {
        assert_eq!(
            flake_uri(&Locked::GitHub {
                owner: "nixos".into(),
                repo: "nixpkgs".into(),
            }),
            "github:nixos/nixpkgs"
        );
        assert_eq!(
            flake_uri(&Locked::GitLab {
                owner: "group".into(),
                repo: "project".into(),
            }),
            "gitlab:group/project"
        );
        assert_eq!(
            flake_uri(&Locked::SourceHut {
                owner: "~user".into(),
                repo: "repo".into(),
            }),
            "sourcehut:~user/repo"
        );
    }

    #[test]
    fn test_flake_uri_url_variants() {
        assert_eq!(
            flake_uri(&Locked::Git {
                url: "https://example.com/repo.git".into(),
            }),
            "git:https://example.com/repo.git"
        );
        assert_eq!(
            flake_uri(&Locked::Hg {
                url: "https://example.com/repo".into(),
            }),
            "hg:https://example.com/repo"
        );
        assert_eq!(
            flake_uri(&Locked::Tarball {
                url: "https://example.com/x.tar.gz".into(),
            }),
            "tarball:https://example.com/x.tar.gz"
        );
        assert_eq!(
            flake_uri(&Locked::File {
                url: "https://example.com/file".into(),
            }),
            "file:https://example.com/file"
        );
    }

    #[test]
    fn test_flake_uri_path_variant() {
        assert_eq!(
            flake_uri(&Locked::Path {
                path: "/some/local/path".into(),
            }),
            "path:/some/local/path"
        );
    }

    #[test]
    fn test_make_scm_uri_lowercases_owner_and_repo() {
        assert_eq!(
            make_scm_uri("github", "NixOS", "Nixpkgs"),
            "github:nixos/nixpkgs"
        );
        assert_eq!(
            make_scm_uri("gitlab", "MIXED-Case", "PROJECT"),
            "gitlab:mixed-case/project"
        );
    }

    #[test]
    fn test_scm_uri_case_insensitive_collision() {
        // Two nodes pointing at the same repo with different casing should
        // collapse to a single uri (and thus be detected as duplicates).
        let lock = r#"
        {
            "nodes": {
                "a": {
                    "locked": {
                        "type": "github",
                        "owner": "NixOS",
                        "repo": "Nixpkgs"
                    }
                },
                "b": {
                    "locked": {
                        "type": "github",
                        "owner": "nixos",
                        "repo": "nixpkgs"
                    }
                },
                "root": {
                    "inputs": {
                        "a": "a",
                        "b": "b"
                    }
                }
            },
            "version": 7,
            "root": "root"
        }
        "#;

        let flake_lock: FlakeLock = serde_json::from_str(lock).unwrap();
        let inputs = parse_inputs(&flake_lock, false);
        let duplicates = find_duplicates(&flake_lock, inputs, &HashSet::new());

        assert_eq!(duplicates.len(), 1);
        assert!(duplicates.contains_key("github:nixos/nixpkgs"));
        assert_eq!(duplicates.get("github:nixos/nixpkgs").unwrap().len(), 2);
    }

    #[test]
    fn test_no_duplicates() {
        let lock = r#"
        {
            "nodes": {
                "a": {
                    "locked": {
                        "type": "github",
                        "owner": "u",
                        "repo": "one"
                    }
                },
                "b": {
                    "locked": {
                        "type": "github",
                        "owner": "u",
                        "repo": "two"
                    }
                },
                "root": {
                    "inputs": {
                        "a": "a",
                        "b": "b"
                    }
                }
            },
            "version": 7,
            "root": "root"
        }
        "#;

        let flake_lock: FlakeLock = serde_json::from_str(lock).unwrap();
        let inputs = parse_inputs(&flake_lock, false);
        let duplicates = find_duplicates(&flake_lock, inputs, &HashSet::new());

        assert!(duplicates.is_empty());
    }

    #[test]
    fn test_find_paths_to_nodes_nested() {
        let lock = r#"
        {
            "nodes": {
                "child": {
                    "locked": {
                        "type": "github",
                        "owner": "u",
                        "repo": "child"
                    }
                },
                "parent": {
                    "inputs": {
                        "child": "child"
                    },
                    "locked": {
                        "type": "github",
                        "owner": "u",
                        "repo": "parent"
                    }
                },
                "root": {
                    "inputs": {
                        "parent": "parent",
                        "child": "child"
                    }
                }
            },
            "version": 7,
            "root": "root"
        }
        "#;

        let flake_lock: FlakeLock = serde_json::from_str(lock).unwrap();
        let paths = find_paths_to_nodes(&flake_lock);

        let parent_paths = paths.get("parent").expect("parent should have a path");
        assert_eq!(parent_paths, &vec!["inputs.parent".to_string()]);

        let mut child_paths = paths.get("child").expect("child should have paths").clone();
        child_paths.sort();
        assert_eq!(
            child_paths,
            vec![
                "inputs.child".to_string(),
                "inputs.parent.inputs.child".to_string(),
            ]
        );

        // The root node itself should not appear in the path map.
        assert!(!paths.contains_key("root"));
    }

    #[test]
    fn test_duplicates_include_dependency_paths() {
        // `child` is reached both directly from root and via `parent`, so the
        // duplicate report should mention both paths.
        let lock = r#"
        {
            "nodes": {
                "child": {
                    "locked": {
                        "type": "github",
                        "owner": "u",
                        "repo": "child"
                    }
                },
                "parent": {
                    "inputs": {
                        "child": "child"
                    },
                    "locked": {
                        "type": "github",
                        "owner": "u",
                        "repo": "parent"
                    }
                },
                "root": {
                    "inputs": {
                        "parent": "parent",
                        "child": "child"
                    }
                }
            },
            "version": 7,
            "root": "root"
        }
        "#;

        let flake_lock: FlakeLock = serde_json::from_str(lock).unwrap();
        let inputs = parse_inputs(&flake_lock, false);
        let duplicates = find_duplicates(&flake_lock, inputs, &HashSet::new());

        let mut entries = duplicates
            .get("github:u/child")
            .expect("child should be reported as a duplicate")
            .clone();
        entries.sort();
        assert_eq!(
            entries,
            vec![
                "child (inputs.child)".to_string(),
                "child (inputs.parent.inputs.child)".to_string(),
            ]
        );
    }

    #[test]
    fn test_parse_inputs_ignores_nodes_without_locked() {
        // The root node has no `locked` field and must be excluded from the
        // parsed inputs map.
        let lock = r#"
        {
            "nodes": {
                "a": {
                    "locked": {
                        "type": "github",
                        "owner": "u",
                        "repo": "r"
                    }
                },
                "root": {
                    "inputs": {
                        "a": "a"
                    }
                }
            },
            "version": 7,
            "root": "root"
        }
        "#;

        let flake_lock: FlakeLock = serde_json::from_str(lock).unwrap();
        let inputs = parse_inputs(&flake_lock, false);

        assert_eq!(inputs.len(), 1);
        assert!(inputs.contains_key("a"));
        assert!(!inputs.contains_key("root"));
    }
}

#[macro_use]
extern crate lazy_static;
extern crate regex;
#[macro_use]
extern crate hyper;
extern crate hubcaps;
extern crate serde_json;
extern crate serde;
extern crate rustc_serialize;
extern crate toml;
#[macro_use]
extern crate log;
extern crate env_logger;

mod error;
use error::SteveError;

mod github;
use github::{PullRequestHook, XGithubEvent};

use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::fmt::Debug;
use std::env;
use std::io::Read;

use hyper::{Client, Server, Url};
use hyper::header::{Headers, Authorization, Bearer, UserAgent};
use hyper::server::{Request, Response};
use hubcaps::{Credentials, Github, IssueOptions};
use serde_json::Value;
use regex::Regex;
use toml::decode_str;

#[derive(RustcEncodable, RustcDecodable)]
struct RepoData {
    pub qa_user: String,
    pub qa_flags: Vec<String>,
}

#[derive(RustcEncodable, RustcDecodable)]
struct ConfigData {
    pub api_root: String,
    pub repos: HashMap<String, RepoData>,
}

macro_rules! panic_log {
    ($($arg:tt)*) => ( {
        error!($($arg)*);
        panic!()
    } );
}

trait ExpectLog {
    type Item;
    fn expect_log(self, &str) -> Self::Item;
}

impl<T> ExpectLog for Option<T> {
    type Item = T;
    fn expect_log(self, msg: &str) -> T {
        match self {
            None => panic_log!("{}", msg),
            Some(s) => s,
        }
    }
}

impl<T, E> ExpectLog for Result<T, E>
    where E: std::fmt::Display
{
    type Item = T;
    fn expect_log(self, msg: &str) -> T {
        match self {
            Err(err) => panic_log!("{} {}", msg, err),
            Ok(s) => s,
        }
    }
}


fn read_config_file(file_name: &str) -> ConfigData {
    let mut config_file = File::open(file_name)
        .expect_log(&format!("Couldn't open config file {}", file_name));
    let mut str = String::new();
    config_file.read_to_string(&mut str)
        .expect_log(&format!("Couldn't read config file {}", file_name));
    let toml: Option<ConfigData> = decode_str(&str);
    toml.expect_log(&format!("Couldn't decode config file {}", file_name))
}

#[test]
fn it_reads_toml() {
    env_logger::init().unwrap();
    let config_data = read_config_file("test-data/test.toml");
    assert_eq!(config_data.api_root, "github.ibm.com/api/v3");
    let steve = config_data.repos.get("lkgrele/steve").unwrap();
    assert_eq!(steve.qa_user, "leif");
    assert_eq!(steve.qa_flags, ["qa", "test"]);
    let raul = config_data.repos.get("raul/robot").unwrap();
    assert_eq!(raul.qa_user, "raul");
    assert_eq!(raul.qa_flags, ["qa"]);
}


fn pr_handler(request: Request, _: Response) {
    let headers = request.headers.clone();
    if let Some(&XGithubEvent(ref event)) = headers.get::<XGithubEvent>() {
        if event == "pull_request" {
            let res = PullRequestHook::from_request(request).map(|pr| {
                pr.run(|&PullRequestHook { ref commits_url, ref owner, ref repo, .. }| {
                    handle_pr(commits_url, repo, owner)
                })
            });
            if res.is_err() {
                error!("Error decoding pull request webhook: {:?}", res.err())
            }
        }
    }
    return ();
}
fn main() {
    env_logger::init().unwrap();

    let ip_and_port = get_ip_and_port();

    match Server::http(ip_and_port.as_str()) {
        Err(err) => error!("ERROR: failed to start server, the error was: {}", err),
        Ok(server) => {
            match server.handle(pr_handler) {
                Err(err) => error!("ERROR starting handler, the error was: {}", err),
                Ok(_) => info!("Successfully started server"),
            }
        }
    }
}

fn env_or(name: &str, def: &str) -> String {
    env::var_os(name).and_then(|v| v.into_string().ok()).unwrap_or(def.to_owned())
}

fn get_ip_and_port() -> String {
    let ip = env_or("STEVE_IP", "0.0.0.0");
    let port = env_or("STEVE_PORT", "80");
    format!("{}:{}", ip, port)
}

fn handle_pr(commits_url: &hyper::Url, repo: &str, owner: &str) {
    let config_data = read_config_file(".steve");
    let auth_token = env::var_os("STEVE_GITHUB_TOKEN")
        .expect_log("Missing github token, STEVE_GITHUB_TOKEN is not defined!")
        .into_string()
        .ok()
        .expect_log("Github token contains illegal characters!");
    let repository = format!("{}/{}", repo, owner);
    match config_data.repos.get(&repository) {
        Some(config) => {
            let _ = update_issues::<String>(&commits_url,
                                            &auth_token,
                                            &config,
                                            &config_data.api_root,
                                            &repo,
                                            &owner)
                .map_err(|err| error!("{:?}", err));
            return ();
        }
        None => info!("No config found for repo {}, skipping", repository),
    }
}

fn do_retry<F, R, E>(func: F, max_retries: u64) -> Result<R, E>
    where F: Fn() -> Result<R, E>,
          R: Debug,
          E: Debug
{
    for _ in 1..(max_retries - 1) {
        let result = func();
        if result.is_ok() {
            return result;
        } else {
            info!("{:?}", result.unwrap_err())
        }
    }
    return func();
}

fn update_issues<E>(commits_url: &hyper::Url,
                    auth_token: &str,
                    config: &RepoData,
                    api_root: &str,
                    repo_name: &str,
                    owner: &str)
                    -> Result<(), SteveError> {
    let client = Client::new();
    let mut headers = Headers::new();
    headers.set(Authorization(Bearer { token: auth_token.to_owned() }));
    headers.set(UserAgent("steve".to_owned()));
    let retries = env_or("STEVE_MAX_RETRIES", "5")
        .parse::<u64>()
        .expect_log("max retries need to be an integer");
    let commits =
        try!(do_retry(|| client.get(commits_url.clone()).headers(headers.clone()).send(),
                      retries));

    let json = try!(serde_json::from_reader(commits));
    let messages = try!(parse_commit_data(&json));
    let mut issues = HashSet::new();
    for message in messages {
        get_issues(&mut issues, &message)
    }

    if !issues.is_empty() {
        let github = Github::host(api_root,
                                  "steve",
                                  &client,
                                  Credentials::Token(auth_token.to_owned()));
        let repo = github.repo(repo_name, owner);
        let issue_data = try!(do_retry(|| repo.issues().list(&Default::default()), retries));
        for issue in issues {
            if issue_data.len() as u64 <= issue {
                info!("Wanted to update issue #{}, but {}/{} doesn't have an issue with that \
                       number",
                      issue,
                      owner,
                      repo_name)
            } else {
                let ref current_flags = issue_data[issue as usize].labels;
                let new_flags: Vec<String> = current_flags.into_iter()
                    .map(|l| l.name.clone())
                    .chain(config.qa_flags.clone().into_iter())
                    .collect();
                let issue_options =
                    IssueOptions::new::<String, String, String, String>(None,
                                                                        None,
                                                                        Some(config.qa_user
                                                                            .clone()),
                                                                        None,
                                                                        Some(new_flags));
                try!(do_retry(|| {
                                  repo.issue(issue)
                                      .edit(&issue_options)
                              },
                              retries));
            }
        }
    }
    return Ok(());
}

#[test]
fn it_parses_commit_data() {
    let file = File::open("test-data/test-messages.json").expect("Couldn't open message file");
    let json = serde_json::from_reader(file);
    assert!(json.is_ok());
    let json = json.unwrap();
    let messages = parse_commit_data(&json);
    assert!(messages.is_ok());
    let messages = messages.unwrap();
    assert_eq!(messages[0], "message1");
    assert_eq!(messages[1], "swordfish");
}


fn parse_commit_data<'a>(json: &'a Value) -> Result<Vec<&'a str>, String> {
    let arr = try!(json.as_array().ok_or("expected array as top level object"));
    arr.iter()
        .map(|obj| {
            obj.pointer("/commit/message")
                .ok_or("missing message field")
                .and_then(|m| m.as_string().ok_or("message field has wrong type"))
                .map_err(|s| s.to_owned())
        })
        .collect()
}


fn update_issue(client: &Client,
                auth_token: &str,
                issue_num: u64,
                config: &RepoData,
                repository: &str,
                api_root: &str) {
    let github = Github::host(api_root,
                              "steve",
                              client,
                              Credentials::Token(auth_token.to_owned()));
    let (owner, repo_name) = match repository.split("/").collect::<Vec<_>>() {
        strs => (strs[0], strs[1]),
    };
    let _ = github.repo(repo_name, owner)
        .issue(issue_num)
        .labels()
        .add(config.qa_flags.iter().map(String::as_str).collect());
    let mut headers = Headers::new();
    headers.set(Authorization(Bearer { token: auth_token.to_owned() }));
    headers.set(UserAgent("steve".to_owned()));
    let update_json = format!("{{\"assignee\":\"{}\"}}", config.qa_user);
    let url = Url::parse(&format!("{}/repos/{}/issues/{}", api_root, repository, issue_num))
        .expect("bad url");
    let res = client.patch(url)
        .body(&update_json)
        .headers(headers.to_owned())
        .send()
        .unwrap();
    assert_eq!(res.status, hyper::Ok);
}

#[test]
fn it_finds_issue_numbers() {
    let message = r"This is a commit message
        needs qa: #72 should get caught
        needs QA #333 should also get caught
        but #44 won't, and needs QA: #33 #45
        only gets #33. Duplicates are discarded:
        needs qa #72";
    let mut expected: HashSet<u64> = HashSet::new();
    expected.insert(72);
    expected.insert(333);
    expected.insert(33);
    let mut numbers: HashSet<u64> = HashSet::new();
    get_issues(&mut numbers, message);
    assert_eq!(numbers, expected);
}

fn get_issues(numbers: &mut HashSet<u64>, message: &str) {
    lazy_static! {
        static ref RE: Regex = Regex::new(r"(?i)needs\sqa:?\s#(\d+)").expect_log("{}");
    }
    for num in RE.captures_iter(message).filter_map(|cap| cap.at(1)) {
        numbers.insert(num.parse::<u64>().expect_log("{}"));
    }
}

#[macro_use]
extern crate lazy_static;
extern crate regex;
extern crate hyper;
extern crate hubcaps;
extern crate afterparty;
extern crate serde_json;
extern crate serde;
extern crate rustc_serialize;
extern crate toml;
#[macro_use]
extern crate log;
extern crate env_logger;

use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::env;
use std::io::Read;

use hyper::{Client, Server, Url};
use hyper::header::{Headers, Authorization, Bearer, UserAgent};
use hubcaps::{Credentials, Github};
use afterparty::*;
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

fn read_config_file(file_name: &str) -> ConfigData {
    let mut config_file = File::open(file_name).expect("Couldn't open config file");
    let mut str = String::new();
    config_file.read_to_string(&mut str).expect("Couldn't read config file");
    let toml: Option<ConfigData> = decode_str(&str);
    match toml {
        Some(cfg) => cfg,
        _ => panic!("Couldn't decode config file"),
    }
}

#[test]
fn it_reads_toml() {
    let config_data = read_config_file("test-data/test.toml");
    assert_eq!(config_data.api_root, "github.ibm.com/api/v3");
    let steve = config_data.repos.get("lkgrele/steve").unwrap();
    assert_eq!(steve.qa_user, "leif");
    assert_eq!(steve.qa_flags, ["qa", "test"]);
    let raul = config_data.repos.get("raul/robot").unwrap();
    assert_eq!(raul.qa_user, "raul");
    assert_eq!(raul.qa_flags, ["qa"]);
}


fn main() {
    env_logger::init().unwrap();

    let mut hub = Hub::new();
    hub.handle("pull_request", |delivery: &Delivery| {
        match delivery.payload {
            Event::PullRequest { ref pull_request, ref repository, .. } => {
                if pull_request.merged {
                    handle_pr(&pull_request.commits_url, &repository.full_name)
                }
            }
            _ => info!("Recived a request that wasn't a pull-request webhook"),
        }
    });

    match Server::http("0.0.0.0:0") {
        Err(err) => error!("ERROR: failed to start server, the error was: {}", err),
        Ok(server) => {
            match server.handle(hub) {
                Err(err) => error!("ERROR starting handler, the error was: {}", err),
                Ok(server) => info!("Successfully started server"),
            }
        }
    }
}


fn handle_pr(commits_url: &str, repository: &str) {
    let config_data = read_config_file(".steve");
    let repo_config = config_data.repos.get(&repository.to_owned());
    // let auth_token = env::var_os("STEVE_GITHUB_TOKEN")
    //     .expect("Missing github token!")
    //     .map(|tok| tok.into_string())
    //     .unwrap();
    match env::var_os("STEVE_GITHUB_TOKEN") {
        None => {
            error!("Missing github token, STEVE_GITHUB_TOKEN is not defined!");
            panic!()
        }
        Some(tok) => {
            match tok.into_string() {
                Err(_) => {
                    error!("Github token contains illegal characters!");
                    panic!()
                }
                Ok(auth_token) => {
                    match repo_config {
                        Some(config) => {
                            update_issues(&commits_url,
                                          &auth_token,
                                          &config,
                                          &config_data.api_root,
                                          &repository)
                        }
                        None => info!("No config found for repo {}, skipping", repository),
                    }
                }
            }
        }
    }
}

fn update_issues(commits_url: &str,
                 auth_token: &str,
                 config: &RepoData,
                 api_root: &str,
                 repository: &str) {
    let client = Client::new();
    let mut headers = Headers::new();
    headers.set(Authorization(Bearer { token: auth_token.to_owned() }));
    headers.set(UserAgent("steve".to_owned()));
    let commits = client.get(commits_url).headers(headers).send().unwrap();
    assert_eq!(commits.status, hyper::Ok); //replace with actual error handling

    match serde_json::from_reader(commits) {
        Ok(json) => {
            match parse_commit_data(&json) {
                Ok(messages) => {
                    let mut issues = HashSet::new();
                    for message in messages {
                        get_issues(&mut issues, &message)
                    }

                    for issue in issues {
                        update_issue(&client, auth_token, issue, config, &repository, api_root);
                    }
                }
                Err(err) => println!("{:?}", err), //should log error
            }
        }
        Err(err) => println!("{:?}", err), //should log error
    }
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


fn parse_commit_data<'a>(json: &'a Value) -> Result<Vec<&'a str>, &str> {
    let arr = try!(json.as_array().ok_or("expected array as top level object"));
    arr.iter()
        .map(|obj| {
            obj.pointer("/commit/message")
                .ok_or("missing message field")
                .and_then(|m| m.as_string().ok_or("message field has wrong type"))
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
        static ref RE: Regex = Regex::new(r"(?i)needs\sqa:?\s#(\d+)").unwrap();
    }
    for num in RE.captures_iter(message).filter_map(|cap| cap.at(1)) {
        numbers.insert(num.parse::<u64>().unwrap());
    }
}

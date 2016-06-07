#[macro_use]
extern crate lazy_static;
extern crate regex;
extern crate hyper;
extern crate hubcaps;
extern crate afterparty;
extern crate serde_json;
extern crate rustc_serialize;
extern crate toml;

use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::env;
use std::io::Read;

use hyper::{Client, Server, Url};
use hyper::header::{Headers, Authorization, Bearer, UserAgent};
use hubcaps::{Credentials, Github};
use afterparty::*;
use serde_json::Value;
use regex::*;
use lazy_static::*;
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

fn read_config_file() -> ConfigData {
    let mut config_file = File::open(".steve").expect("Couldn't open config file");
    let mut str = String::new();
    config_file.read_to_string(&mut str).expect("Couldn't read config file");
    let toml: Option<ConfigData> = decode_str(&str);
    match toml {
        Some(cfg) => cfg,
        _ => panic!("Couldn't decode config file"),
    }
}


fn main() {
    let mut hub = Hub::new();
    hub.handle("pull_request", |delivery: &Delivery| {
        match delivery.payload {
            Event::PullRequest { number, ref pull_request, ref repository, .. } => {
                if pull_request.merged {
                    handle_pr(number, &pull_request.commits_url, &repository.full_name)
                }
            }
            _ => (),
        }
    });

    Server::http("0.0.0.0").unwrap().handle(hub).unwrap();
}

fn handle_pr(number: u64, commits_url: &str, repository: &str) {
    let config_data = read_config_file();
    let repo_config = config_data.repos.get(&repository.to_owned());
    let auth_token = env::var_os("STEVE_GITHUB_TOKEN")
        .expect("Missing github token!")
        .into_string()
        .unwrap();

    match repo_config {
        Some(config) => {
            update_issues(number,
                          &commits_url,
                          &auth_token,
                          &config,
                          &config_data.api_root,
                          &repository)
        }
        None => (), //just don't do anything if we don't have a config for this repo
    }
}

fn update_issues(number: u64,
                 commits_url: &str,
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

    let data: Value = serde_json::from_reader(commits).unwrap();
    let arr = data.as_array().unwrap();
    let mut issues = HashSet::new();
    for val in arr {
        // needs real error handling, shouldn't panic when it gets bad json
        let message = val.as_object().unwrap().get("message").unwrap().as_string().unwrap();
        get_issues(&mut issues, &message);
    }

    for issue in issues {
        update_issue(&client, auth_token, issue, config, &repository, api_root);
    }
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
    github.repo(repo_name, owner)
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

fn get_issues(numbers: &mut HashSet<u64>, message: &str) {
    lazy_static! {
        static ref RE: Regex = Regex::new(r"(?i)needs\sqa:?\s#(\d+)").unwrap();
    }
    for cap in RE.captures_iter(message) {
        if let Some(num) = cap.at(1) {
            numbers.insert(num.parse::<u64>().unwrap());
        }
    }
}

extern crate hyper;
extern crate serde;
extern crate serde_json;


use error::SteveError;
use hyper::server::Request;


pub struct PullRequestHook {
    pub commits_url: hyper::Url,
    pub owner: String,
    pub repo: String,
    pub was_merged: bool,
}

fn from_pointer<T>(obj: &serde_json::Value, pointer: &str) -> Result<T, SteveError>
    where T: serde::Deserialize
{
    let val = try!(obj.pointer(pointer).ok_or(format!("Couldn't find field {}", pointer)));
    serde_json::from_value::<T>(val.clone()).map_err(SteveError::from)
}


header! {(XGithubEvent, "X-Github-Event") => [String]}

impl PullRequestHook {
    pub fn from_request(request: Request) -> Result<Self, SteveError> {
        let obj: serde_json::Value = try!(serde_json::from_reader(request));
        let commits_url = try!(from_pointer::<String>(&obj, "/pull_request/commits_url")
            .and_then(|s| hyper::Url::parse(&s).map_err(|_| SteveError::from("url parse error"))));
        let owner = try!(from_pointer(&obj, "/pull_request/repo/owner/login"));
        let repo = try!(from_pointer(&obj, "/pull_request/repo/name"));
        let was_merged = try!(from_pointer(&obj, "/pull_request/merged"));
        Ok(PullRequestHook {
            commits_url: commits_url,
            owner: owner,
            repo: repo,
            was_merged: was_merged,
        })
    }

    pub fn run<F>(&self, func: F) -> ()
        where F: Fn(&PullRequestHook) -> ()
    {
        if self.was_merged {
            func(&self)
        }
    }
}

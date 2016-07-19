extern crate serde_json;
extern crate hyper;
extern crate hubcaps;

use std::error::Error;
use std::fmt;

use hyper::error::Error as HypErr;
use serde_json::error::Error as SjErr;
use hubcaps::errors::Error as HubErr;

#[derive(Debug)]
pub struct SteveError {
    error_message: String,
}

impl Error for SteveError {
    fn description(&self) -> &str {
        self.error_message.as_str()
    }
}

impl fmt::Display for SteveError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}", self.error_message)
    }
}

impl From<HypErr> for SteveError {
    fn from(err: HypErr) -> Self {
        SteveError { error_message: format!("{:?}", err) }
    }
}

impl From<SjErr> for SteveError {
    fn from(err: SjErr) -> Self {
        SteveError { error_message: format!("{:?}", err) }
    }
}

impl From<HubErr> for SteveError {
    fn from(err: HubErr) -> Self {
        SteveError { error_message: format!("{:?}", err) }
    }
}

impl From<String> for SteveError {
    fn from(err: String) -> Self {
        SteveError { error_message: err }
    }
}

impl From<&'static str> for SteveError {
    fn from(err: &str) -> Self {
        SteveError { error_message: err.to_owned() }
    }
}

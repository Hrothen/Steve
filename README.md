Steve handles github webhook integrations.

This is a work in progress, it doesn't have enough tests, and testing against
github webhooks is a pain so that part may not get automated.

#Usage

Steve looks for the following environment variables:

* `STEVE_GITHUB_TOKEN` specifies the github api token to use, this must be set.
* `STEVE_IP` defaults to `0.0.0.0` if unset
* `STEVE_PORT` defaults to `80` if unset.

Per-repo configuration is stored in the `.steve` file, which is a `toml` file with the following layout:

```toml
api_root = <api root for github> #this is github.com/api/v3 if you're not using github enterprise

[repos]

[repos."<owner>/<repo name>"]
qa_flags = ["foo", "bar"]
qa_user = <user to assign to QA tasks>
```

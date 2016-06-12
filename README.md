Steve handles github webhook integrations.

This is a work in progress, it doesn't have enough tests, and testing against
github webhooks is a pain so that part may not get automated.

#Building

You need to checkout a local copy of [afterparty](https://github.com/softprops/afterparty) and bump its hyper version to 0.9
change the path in Steve's `Cargo.toml` to point at wherever you checked it out.

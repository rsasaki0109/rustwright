//! Small argument parser shared by the introductory browser examples.
use std::{io, path::PathBuf};

pub struct RunOptions {
    pub headless: bool,
    pub url: String,
    pub profile: Option<PathBuf>,
    pub screenshot: PathBuf,
}

impl RunOptions {
    pub fn parse(example: &str, default_screenshot: &str) -> io::Result<Option<Self>> {
        let mut options = Self {
            headless: false,
            url: "https://example.com/".into(),
            profile: None,
            screenshot: default_screenshot.into(),
        };
        let mut supplied_url = false;
        let mut args = std::env::args().skip(1);
        while let Some(arg) = args.next() {
            match arg.as_str() {
                "--help" | "-h" => {
                    println!(
                        "Usage: {example} [--headless] [--profile PATH] [--screenshot PATH] [--url URL | URL]\n\
                         Defaults: visible browser, https://example.com/, screenshot {default_screenshot}."
                    );
                    return Ok(None);
                }
                "--headless" => options.headless = true,
                "--profile" => options.profile = Some(value(&mut args, "--profile")?.into()),
                "--screenshot" => options.screenshot = value(&mut args, "--screenshot")?.into(),
                "--url" => {
                    if supplied_url {
                        return Err(invalid("supply only one URL"));
                    }
                    options.url = value(&mut args, "--url")?;
                    supplied_url = true;
                }
                other if other.starts_with('-') => {
                    return Err(invalid(format!("unknown option: {other}")));
                }
                other => {
                    if supplied_url || other.is_empty() {
                        return Err(invalid("supply only one nonempty URL"));
                    }
                    options.url = other.to_owned();
                    supplied_url = true;
                }
            }
        }
        Ok(Some(options))
    }
}

fn value(args: &mut impl Iterator<Item = String>, option: &str) -> io::Result<String> {
    args.next()
        .filter(|value| !value.is_empty() && !value.starts_with('-'))
        .ok_or_else(|| invalid(format!("{option} requires a value")))
}

fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, message.into())
}

//! Other apps asking Convertino to open its wheel for files they show
//! (Crofty's Dora explorer, where the Alt+right-click hook doesn't act).
//!
//!   Convertino.exe --wheel [--at X,Y] [--from NAME] -- <file or folder>…
//!
//! X,Y is where to centre the wheel: physical screen pixels on Windows,
//! points on macOS. Without --at the wheel opens at the mouse pointer.
//! A second launch hands its arguments to the running Convertino (single
//! instance) and quits; if Convertino isn't running, it starts and opens the
//! wheel once it's ready. Listed as "wheel" under "can" in the shared tools
//! folder entry (siblings.rs), so callers know this copy understands it.

/// A request to open the wheel, read from a command line.
#[derive(Debug, Clone, PartialEq)]
pub struct WheelRequest {
    pub paths: Vec<String>,
    pub at: Option<(f64, f64)>,
    pub from: String,
}

/// The wheel request in these arguments (the first one is the program), if any.
pub fn wheel_request(args: &[String]) -> Option<WheelRequest> {
    let mut it = args.iter().skip(1);
    let mut wheel = false;
    let mut at = None;
    let mut from = String::from("another app");
    let mut paths = Vec::new();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--wheel" => wheel = true,
            "--at" => at = it.next().and_then(|v| parse_point(v)),
            "--from" => {
                if let Some(v) = it.next() {
                    if !v.trim().is_empty() {
                        from = v.trim().to_string();
                    }
                }
            }
            "--" => {
                paths.extend(it.by_ref().filter(|p| !p.is_empty()).cloned());
                break;
            }
            s if s.starts_with("--") => {} // other switches (--background) aren't ours
            _ => paths.push(a.clone()),
        }
    }
    (wheel && !paths.is_empty()).then_some(WheelRequest { paths, at, from })
}

fn parse_point(v: &str) -> Option<(f64, f64)> {
    let (x, y) = v.split_once(',')?;
    let (x, y) = (x.trim().parse::<f64>().ok()?, y.trim().parse::<f64>().ok()?);
    (x.is_finite() && y.is_finite()).then_some((x, y))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn args(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn full_request() {
        let r = wheel_request(&args(&["C:\\Convertino.exe", "--wheel", "--at", "1200,-40", "--from", "Dora", "--", "C:\\a b.png", "--odd-name.jpg"])).unwrap();
        assert_eq!(r.paths, vec!["C:\\a b.png", "--odd-name.jpg"]);
        assert_eq!(r.at, Some((1200.0, -40.0)));
        assert_eq!(r.from, "Dora");
    }

    #[test]
    fn without_at_or_separator() {
        let r = wheel_request(&args(&["convertino", "--wheel", "/x/one.mov"])).unwrap();
        assert_eq!(r.paths, vec!["/x/one.mov"]);
        assert_eq!(r.at, None);
        assert_eq!(r.from, "another app");
    }

    #[test]
    fn not_a_wheel_request() {
        assert_eq!(wheel_request(&args(&["convertino"])), None);
        assert_eq!(wheel_request(&args(&["convertino", "--background"])), None);
        assert_eq!(wheel_request(&args(&["convertino", "--wheel"])), None, "no files");
        assert_eq!(wheel_request(&args(&["convertino", "C:\\file.png"])), None, "no --wheel");
    }

    #[test]
    fn bad_point_is_ignored() {
        let r = wheel_request(&args(&["c", "--wheel", "--at", "nope", "--", "f.png"])).unwrap();
        assert_eq!(r.at, None);
        assert_eq!(r.paths, vec!["f.png"]);
    }
}

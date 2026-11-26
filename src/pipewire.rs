use crate::alsa_ctl::CardEntry;
use serde::Serialize;
use std::io::Read;
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

const PW_LINK_TIMEOUT: Duration = Duration::from_millis(500);

#[derive(Debug, Clone, Serialize)]
pub struct PwLink {
    pub id: u32,
    pub output_port: String,
    pub input_port: String,
    pub is_scarlett: bool,
}

fn run_pw_link_bounded(args: &[&str], timeout: Duration) -> Result<String, String> {
    let mut child = Command::new("pw-link")
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("failed to run pw-link: {e}"))?;

    let stdout_pipe = child.stdout.take();
    let stderr_pipe = child.stderr.take();

    let out_thread = thread::spawn(move || {
        let mut buf = Vec::new();
        if let Some(mut s) = stdout_pipe {
            let _ = s.read_to_end(&mut buf);
        }
        buf
    });
    let err_thread = thread::spawn(move || {
        let mut buf = Vec::new();
        if let Some(mut s) = stderr_pipe {
            let _ = s.read_to_end(&mut buf);
        }
        buf
    });

    let start = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(st)) => break st,
            Ok(None) => {
                if start.elapsed() >= timeout {
                    let _ = child.kill();
                    let _ = child.wait();
                    let _ = out_thread.join();
                    let _ = err_thread.join();
                    return Err("pw-link timed out".into());
                }
                thread::sleep(Duration::from_millis(10));
            }
            Err(e) => {
                let _ = child.kill();
                let _ = child.wait();
                let _ = out_thread.join();
                let _ = err_thread.join();
                return Err(format!("pw-link wait failed: {e}"));
            }
        }
    };

    let stdout = out_thread.join().unwrap_or_default();
    let stderr = err_thread.join().unwrap_or_default();
    if !status.success() {
        return Err(String::from_utf8_lossy(&stderr).trim().to_string());
    }
    Ok(String::from_utf8_lossy(&stdout).into_owned())
}

pub fn list_links_for_card(card: Option<&CardEntry>) -> Result<Vec<PwLink>, String> {
    let text = run_pw_link_bounded(&["-l", "-I"], PW_LINK_TIMEOUT)?;
    Ok(parse_pw_links_for_card(&text, card))
}

fn port_matches_focusrite(port: &str, card: Option<&CardEntry>) -> bool {
    let lower = port.to_ascii_lowercase();
    let generic_focusrite = lower.contains("focusrite")
        || lower.contains("scarlett")
        || lower.contains("clarett")
        || lower.contains("vocaster");
    if !generic_focusrite {
        return false;
    }
    let Some(card) = card else {
        return true;
    };

    // Match model tokens (e.g. "4i4", "2i2", "18i20", "clarett", "vocaster") so
    // multiple Focusrite interfaces on the same host filter to the active card.
    let norm_port = lower.replace(['-', ' '], "_");
    let tokens: Vec<String> = card
        .name
        .to_ascii_lowercase()
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|t| {
            t.len() >= 2
                && !matches!(
                    *t,
                    "usb" | "focusrite" | "audio" | "gen" | "2nd" | "3rd" | "4th"
                )
        })
        .map(String::from)
        .collect();

    if tokens.is_empty() {
        return true;
    }
    tokens.iter().all(|tok| norm_port.contains(tok))
}

fn parse_pw_links_for_card(text: &str, card: Option<&CardEntry>) -> Vec<PwLink> {
    let mut links = Vec::new();
    let mut current_port: Option<String> = None;

    for raw in text.lines() {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            continue;
        }
        if let Some((left, right)) = trimmed.split_once("|->") {
            if let Some(ref src) = current_port {
                let link_id = left.trim().parse::<u32>().unwrap_or(0);
                let dst = right
                    .trim()
                    .split_once(char::is_whitespace)
                    .map(|(_, name)| name.trim().to_string())
                    .unwrap_or_else(|| right.trim().to_string());
                if link_id > 0 {
                    let is_scarlett =
                        port_matches_focusrite(src, card) || port_matches_focusrite(&dst, card);
                    links.push(PwLink {
                        id: link_id,
                        output_port: src.clone(),
                        input_port: dst,
                        is_scarlett,
                    });
                }
            }
        } else if trimmed.contains("|<-") {
            // Reverse direction duplicate of the same link; ignore.
        } else {
            // Port header line (ID may be right-aligned with leading spaces):
            // "  77 alsa_input.usb-Focusrite_..."
            let port_name = trimmed
                .split_once(char::is_whitespace)
                .map(|(_, name)| name.trim().to_string())
                .unwrap_or_else(|| trimmed.to_string());
            current_port = Some(port_name);
        }
    }

    // Sort active Focusrite card links first, then by output/input port name.
    links.sort_by(|a, b| {
        b.is_scarlett
            .cmp(&a.is_scarlett)
            .then_with(|| a.output_port.cmp(&b.output_port))
            .then_with(|| a.input_port.cmp(&b.input_port))
    });
    links
}

pub fn disconnect_link(link_id: u32) -> Result<(), String> {
    let id_str = link_id.to_string();
    run_pw_link_bounded(&["-d", &id_str], PW_LINK_TIMEOUT).map(|_| ())
}

pub fn short_port(port: &str) -> String {
    if let Some((node, chan)) = port.split_once(':') {
        let lower = node.to_ascii_lowercase();
        let family = if lower.contains("scarlett") {
            Some("Scarlett")
        } else if lower.contains("clarett") {
            Some("Clarett")
        } else if lower.contains("vocaster") {
            Some("Vocaster")
        } else {
            None
        };
        let short_node = if let Some(fam) = family {
            if node.starts_with("alsa_input.") {
                format!("{fam} In")
            } else if node.starts_with("alsa_output.") {
                format!("{fam} Out")
            } else {
                node.to_string()
            }
        } else if node.starts_with("alsa_output.") || node.starts_with("alsa_input.") {
            node.rsplit('.').next().unwrap_or(node).to_string()
        } else {
            node.to_string()
        };
        format!("{short_node}:{chan}")
    } else {
        port.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_pw_link_l_i_output_and_filters_by_card() {
        let sample = "\
  77 alsa_input.usb-Focusrite_Scarlett_4i4_USB_TEST-00.analog-surround-21:capture_FR
 106   |->   68 alsa_output.usb-Focusrite_Scarlett_4i4_USB_TEST-00.analog-surround-40:playback_FL
 127   |->   69 alsa_output.usb-Focusrite_Scarlett_4i4_USB_TEST-00.analog-surround-40:playback_FR
  88 alsa_input.usb-Focusrite_Vocaster_Two_USB_TEST-00.analog-stereo:capture_FL
 201   |->   90 alsa_output.usb-Focusrite_Vocaster_Two_USB_TEST-00.analog-stereo:playback_FL
";
        let card_4i4 = CardEntry {
            index: 1,
            alsa_id: "USB".into(),
            name: "Scarlett 4i4 USB".into(),
            usb_id: "1235:8212".into(),
            generation: "Gen 3".into(),
        };
        let links = parse_pw_links_for_card(sample, Some(&card_4i4));
        assert_eq!(links.len(), 3);
        assert_eq!(links.iter().filter(|l| l.is_scarlett).count(), 2);
        assert_eq!(links[0].id, 106);
        assert_eq!(short_port(&links[0].output_port), "Scarlett In:capture_FR");
        assert_eq!(short_port(&links[0].input_port), "Scarlett Out:playback_FL");

        let card_vocaster = CardEntry {
            index: 2,
            alsa_id: "Vocaster".into(),
            name: "Vocaster Two USB".into(),
            usb_id: "1235:8217".into(),
            generation: "Vocaster".into(),
        };
        let v_links = parse_pw_links_for_card(sample, Some(&card_vocaster));
        assert_eq!(v_links.iter().filter(|l| l.is_scarlett).count(), 1);
        assert_eq!(
            short_port(&v_links[0].output_port),
            "Vocaster In:capture_FL"
        );
    }
}

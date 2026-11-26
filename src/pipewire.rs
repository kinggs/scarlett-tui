use serde::Serialize;
use std::process::Command;

#[derive(Debug, Clone, Serialize)]
pub struct PwLink {
    pub id: u32,
    pub output_port: String,
    pub input_port: String,
    pub is_scarlett: bool,
}

pub fn list_links() -> Result<Vec<PwLink>, String> {
    let out = Command::new("pw-link")
        .args(["-l", "-I"])
        .output()
        .map_err(|e| format!("failed to run pw-link: {e}"))?;
    if !out.status.success() {
        return Err(String::from_utf8_lossy(&out.stderr).trim().to_string());
    }
    let text = String::from_utf8_lossy(&out.stdout);
    Ok(parse_pw_links(&text))
}

fn parse_pw_links(text: &str) -> Vec<PwLink> {
    let mut links = Vec::new();
    let mut current_port: Option<String> = None;

    for raw in text.lines() {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            continue;
        }
        if !raw.starts_with(' ') && !raw.starts_with('\t') {
            // Port header line: "<port_id> <port_name>"
            let port_name = trimmed
                .split_once(char::is_whitespace)
                .map(|(_, name)| name.trim().to_string())
                .unwrap_or_else(|| trimmed.to_string());
            current_port = Some(port_name);
        } else if let Some(rest) = trimmed.strip_prefix("|->") {
            // Link line: "<link_id> |-> <target_port_id> <target_port_name>" or "|-> ..."
            // Wait: with `pw-link -l -I`, format is: "  <link_id> |-> <port_id> <port_name>"
            // Handled below via split_once("|->").
            let _ = rest;
        } else if let Some((left, right)) = trimmed.split_once("|->")
            && let Some(ref src) = current_port
        {
            let link_id = left.trim().parse::<u32>().unwrap_or(0);
            let dst = right
                .trim()
                .split_once(char::is_whitespace)
                .map(|(_, name)| name.trim().to_string())
                .unwrap_or_else(|| right.trim().to_string());
            if link_id > 0 {
                let is_scarlett = src.to_ascii_lowercase().contains("scarlett")
                    || dst.to_ascii_lowercase().contains("scarlett");
                links.push(PwLink {
                    id: link_id,
                    output_port: src.clone(),
                    input_port: dst,
                    is_scarlett,
                });
            }
        }
    }

    // Sort Scarlett links first, then by output/input port name.
    links.sort_by(|a, b| {
        b.is_scarlett
            .cmp(&a.is_scarlett)
            .then_with(|| a.output_port.cmp(&b.output_port))
            .then_with(|| a.input_port.cmp(&b.input_port))
    });
    links
}

pub fn disconnect_link(link_id: u32) -> Result<(), String> {
    let out = Command::new("pw-link")
        .args(["-d", &link_id.to_string()])
        .output()
        .map_err(|e| format!("failed to run pw-link -d: {e}"))?;
    if !out.status.success() {
        return Err(String::from_utf8_lossy(&out.stderr).trim().to_string());
    }
    Ok(())
}

pub fn short_port(port: &str) -> String {
    if let Some((node, chan)) = port.split_once(':') {
        let short_node = if node.contains("Scarlett_4i4") || node.contains("Scarlett") {
            if node.starts_with("alsa_input.") {
                "Scarlett In"
            } else if node.starts_with("alsa_output.") {
                "Scarlett Out"
            } else {
                node
            }
        } else if node.starts_with("alsa_output.") || node.starts_with("alsa_input.") {
            node.rsplit('.').next().unwrap_or(node)
        } else {
            node
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
    fn parses_pw_link_l_i_output() {
        let sample = "\
77 alsa_input.usb-Focusrite_Scarlett_4i4_USB_TEST-00.analog-surround-21:capture_FR
  106 |-> 68 alsa_output.usb-Focusrite_Scarlett_4i4_USB_TEST-00.analog-surround-40:playback_FL
  127 |-> 69 alsa_output.usb-Focusrite_Scarlett_4i4_USB_TEST-00.analog-surround-40:playback_FR
68 alsa_output.usb-Focusrite_Scarlett_4i4_USB_TEST-00.analog-surround-40:playback_FL
  106 |<- 77 alsa_input.usb-Focusrite_Scarlett_4i4_USB_TEST-00.analog-surround-21:capture_FR
";
        let links = parse_pw_links(sample);
        assert_eq!(links.len(), 2);
        assert_eq!(links[0].id, 106);
        assert!(links[0].is_scarlett);
        assert_eq!(short_port(&links[0].output_port), "Scarlett In:capture_FR");
        assert_eq!(short_port(&links[0].input_port), "Scarlett Out:playback_FL");
    }
}

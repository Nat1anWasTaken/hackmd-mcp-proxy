use crate::store::GitHubUser;

pub(super) fn render_token_form(user: &GitHubUser, return_to: &str, error: Option<&str>) -> String {
    let error_html = error
        .map(|error| format!(r#"<p class="error">{}</p>"#, escape_html(error)))
        .unwrap_or_default();
    format!(
        r#"<!doctype html>
<html lang="en">
<head>
  <meta charset="utf-8">
  <meta name="viewport" content="width=device-width, initial-scale=1">
  <title>Connect HackMD</title>
  <style>{}</style>
</head>
<body>
  <main>
    <h1>Connect HackMD</h1>
    <p>Signed in with GitHub as <strong>{}</strong>.</p>
    <p>Paste a HackMD API token. It will be verified, encrypted, and stored for this GitHub user.</p>
    {}
    <form method="post" action="/hackmd/token">
      <input type="hidden" name="return_to" value="{}">
      <label>HackMD API Token
        <input name="hackmd_api_token" type="password" autocomplete="off" required autofocus>
      </label>
      <button type="submit">Save and continue</button>
    </form>
  </main>
</body>
</html>"#,
        page_css(),
        escape_html(&user.github_login),
        error_html,
        escape_html(return_to)
    )
}

pub(super) fn render_settings(user: &GitHubUser, fingerprint: Option<&str>) -> String {
    let status = fingerprint
        .map(|fingerprint| {
            format!(
                "Connected. Token fingerprint: <code>{}</code>",
                escape_html(fingerprint)
            )
        })
        .unwrap_or_else(|| "Not connected.".to_owned());
    format!(
        r#"<!doctype html>
<html lang="en">
<head>
  <meta charset="utf-8">
  <meta name="viewport" content="width=device-width, initial-scale=1">
  <title>HackMD MCP Settings</title>
  <style>{}</style>
</head>
<body>
  <main>
    <h1>HackMD MCP Settings</h1>
    <p>Signed in with GitHub as <strong>{}</strong>.</p>
    <p>{}</p>
    <form method="post" action="/hackmd/token">
      <input type="hidden" name="return_to" value="/settings">
      <label>Update HackMD API Token
        <input name="hackmd_api_token" type="password" autocomplete="off" required>
      </label>
      <button type="submit">Save token</button>
    </form>
    <form method="post" action="/settings/disconnect">
      <button class="secondary" type="submit">Disconnect HackMD</button>
    </form>
  </main>
</body>
</html>"#,
        page_css(),
        escape_html(&user.github_login),
        status
    )
}

fn page_css() -> &'static str {
    "body{font-family:system-ui,-apple-system,BlinkMacSystemFont,'Segoe UI',sans-serif;margin:0;background:#f7f7f5;color:#151515}main{max-width:560px;margin:12vh auto;padding:32px;background:#fff;border:1px solid #ddd;border-radius:8px}label{display:block;margin:24px 0 12px;font-weight:600}input{display:block;width:100%;box-sizing:border-box;margin-top:8px;padding:10px;border:1px solid #aaa;border-radius:6px;font:inherit}button{padding:10px 14px;border:0;border-radius:6px;background:#166534;color:white;font:inherit;font-weight:600;cursor:pointer}.secondary{margin-top:16px;background:#555}.error{padding:10px 12px;background:#fee2e2;border:1px solid #fecaca;border-radius:6px;color:#991b1b}code{background:#eee;padding:2px 5px;border-radius:4px}"
}

fn escape_html(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

#[cfg(test)]
mod tests {
    use super::escape_html;

    #[test]
    fn escapes_html() {
        assert_eq!(escape_html("<x>&\"'"), "&lt;x&gt;&amp;&quot;&#39;");
    }
}

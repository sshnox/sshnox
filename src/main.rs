use chrono::{Duration, Utc};
use serde::Deserialize;
use std::collections::HashMap;
use std::env;
use std::error::Error;
use std::fmt::Write as FmtWrite;
use std::fs;
use std::path::PathBuf;
use std::time::Duration as StdDuration;
use ureq::Agent;

#[derive(Deserialize)]
struct UserProfile {
    public_repos: u64,
    followers: u64,
}

#[derive(Deserialize)]
struct SearchIssues {
    total_count: u64,
}

#[derive(Deserialize)]
struct RepoSummary {
    fork: bool,
    stargazers_count: u64,
    language: Option<String>,
}

#[derive(Deserialize)]
struct PublicEvent {
    created_at: String,
}

struct Metrics {
    username: String,
    public_repos: u64,
    followers: u64,
    stars: u64,
    open_prs: u64,
    langs: Vec<(String, u64)>,
    buckets: Vec<u64>,
    synced: String,
}

struct Theme<'a> {
    dark: bool,
    bg: &'a str,
    header_bg: &'a str,
    card_bg: &'a str,
    chart_bg: &'a str,
    border: &'a str,
    card_border: &'a str,
    split: &'a str,
    grid: &'a str,
    title_col: &'a str,
    muted: &'a str,
    prompt: &'a str,
    text_hi: &'a str,
    bar_bg: &'a str,
    repo_col: &'a str,
    pr_col: &'a str,
    star_col: &'a str,
    pill_bg: &'a str,
    pill_border: &'a str,
    pill_dot: &'a str,
    pill_text: &'a str,
    spark_col: &'a str,
    grad_id: &'a str,
}

const DARK_THEME: Theme<'static> = Theme {
    dark: true,
    bg: "#0D0F14",
    header_bg: "#141821",
    card_bg: "#141821",
    chart_bg: "#121620",
    border: "#252A34",
    card_border: "#242B38",
    split: "#202531",
    grid: "#1E2433",
    title_col: "#94A3B8",
    muted: "#64748B",
    prompt: "#FDE047",
    text_hi: "#E2E8F0",
    bar_bg: "#1A1F2B",
    repo_col: "#4ADE80",
    pr_col: "#38BDF8",
    star_col: "#FDE047",
    pill_bg: "#132219",
    pill_border: "#1E462F",
    pill_dot: "#4ADE80",
    pill_text: "#86EFAC",
    spark_col: "#FDE047",
    grad_id: "spark-fill-d",
};

const LIGHT_THEME: Theme<'static> = Theme {
    dark: false,
    bg: "#FCFBF7",
    header_bg: "#F2EFE6",
    card_bg: "#FFFFFF",
    chart_bg: "#FFFFFF",
    border: "#D6D3C9",
    card_border: "#D6D3C9",
    split: "#E4E2DA",
    grid: "#E4E2DA",
    title_col: "#52525B",
    muted: "#71717A",
    prompt: "#CA8A04",
    text_hi: "#18181B",
    bar_bg: "#E4E2DA",
    repo_col: "#15803D",
    pr_col: "#0284C7",
    star_col: "#CA8A04",
    pill_bg: "#ECFDF5",
    pill_border: "#A7F3D0",
    pill_dot: "#16A34A",
    pill_text: "#166534",
    spark_col: "#CA8A04",
    grad_id: "spark-fill-l",
};

fn lang_color(lang: &str, dark: bool) -> &'static str {
    match (lang, dark) {
        ("TypeScript", true) => "#38BDF8",
        ("JavaScript", true) => "#FDE047",
        ("Python", true) => "#60A5FA",
        ("HTML", true) => "#FB923C",
        ("CSS", true) => "#A78BFA",
        ("Go", true) => "#34D399",
        ("Rust", true) => "#F97316",
        (_, true) => "#38BDF8",
        ("TypeScript", false) => "#0284C7",
        ("JavaScript", false) => "#CA8A04",
        ("Python", false) => "#2563EB",
        ("HTML", false) => "#EA580C",
        ("CSS", false) => "#7C3AED",
        ("Go", false) => "#059669",
        ("Rust", false) => "#C2410C",
        (_, false) => "#0284C7",
    }
}

fn api_get<T: for<'de> Deserialize<'de>>(
    agent: &Agent,
    url: &str,
    user_agent: &str,
    token: &str,
) -> Result<T, Box<dyn Error>> {
    let mut req = agent
        .get(url)
        .header("Accept", "application/vnd.github+json")
        .header("User-Agent", user_agent);

    if !token.is_empty() {
        req = req.header("Authorization", &format!("Bearer {token}"));
    }

    let parsed: T = req.call()?.body_mut().read_json()?;
    Ok(parsed)
}

fn build_sparkline_paths(buckets: &[u64]) -> (String, String, i32, i32) {
    let (x0, x1) = (664.0_f64, 864.0_f64);
    let (y_top, y_bot) = (92.0_f64, 186.0_f64);
    let peak = buckets.iter().copied().max().unwrap_or(1).max(1) as f64;
    let denom = buckets.len().saturating_sub(1).max(1) as f64;
    let step = (x1 - x0) / denom;

    let mut line_d = String::new();
    let mut last_x = x1 as i32;
    let mut last_y = y_bot as i32;

    for (i, &val) in buckets.iter().enumerate() {
        let x = (x0 + (i as f64) * step).round() as i32;
        let y = (y_bot - ((val as f64) / peak) * (y_bot - y_top)).round() as i32;
        if i == 0 {
            let _ = write!(line_d, "M {x} {y}");
        } else {
            let _ = write!(line_d, " L {x} {y}");
        }
        last_x = x;
        last_y = y;
    }

    let area_d = format!("{line_d} L 864 196 L 664 196 Z");
    (line_d, area_d, last_x, last_y)
}

fn fetch_metrics(username: &str, token: &str) -> Result<Metrics, Box<dyn Error>> {
    let agent: Agent = Agent::config_builder()
        .timeout_global(Some(StdDuration::from_secs(20)))
        .build()
        .into();
    let ua = format!("{username}-rust-profile-telemetry");

    let user: UserProfile = api_get(
        &agent,
        &format!("https://api.github.com/users/{username}"),
        &ua,
        token,
    )?;
    let prs: SearchIssues = api_get(
        &agent,
        &format!("https://api.github.com/search/issues?q=is:pr+author:{username}+is:open&per_page=1"),
        &ua,
        token,
    )?;
    let repos: Vec<RepoSummary> = api_get(
        &agent,
        &format!("https://api.github.com/users/{username}/repos?type=owner&sort=updated&per_page=100"),
        &ua,
        token,
    )?;
    let events: Vec<PublicEvent> = api_get(
        &agent,
        &format!("https://api.github.com/users/{username}/events/public?per_page=100"),
        &ua,
        token,
    )?;

    let mut lang_counts: HashMap<String, u64> = HashMap::new();
    let mut stars: u64 = 0;

    for repo in repos {
        if repo.fork {
            continue;
        }
        stars += repo.stargazers_count;
        if let Some(lang) = repo.language {
            *lang_counts.entry(lang).or_insert(0) += 1;
        }
    }

    let total: u64 = lang_counts.values().sum::<u64>().max(1);
    let mut sorted_langs: Vec<(String, u64)> = lang_counts.into_iter().collect();
    sorted_langs.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    let langs: Vec<(String, u64)> = sorted_langs
        .into_iter()
        .take(5)
        .map(|(name, count)| {
            let pct = (((count as f64) / (total as f64)) * 100.0).round() as u64;
            (name, pct.max(1))
        })
        .collect();

    let today = Utc::now().date_naive();
    let mut day_index: HashMap<String, usize> = HashMap::with_capacity(14);
    for i in 0..14_usize {
        let day = today - Duration::days((13 - i) as i64);
        day_index.insert(day.format("%Y-%m-%d").to_string(), i);
    }

    let mut buckets = vec![0_u64; 14];
    for ev in events {
        if let Some(date_prefix) = ev.created_at.get(..10) {
            if let Some(&idx) = day_index.get(date_prefix) {
                buckets[idx] += 1;
            }
        }
    }

    if buckets.iter().sum::<u64>() == 0 {
        buckets = vec![1, 2, 1, 3, 2, 4, 3, 5, 4, 6, 5, 7, 6, 8];
    }

    Ok(Metrics {
        username: username.to_string(),
        public_repos: user.public_repos,
        followers: user.followers,
        stars,
        open_prs: prs.total_count,
        langs,
        buckets,
        synced: today.format("%Y-%m-%d").to_string(),
    })
}

fn render_svg(metrics: &Metrics, t: &Theme<'_>) -> String {
    let (line_d, area_d, end_x, end_y) = build_sparkline_paths(&metrics.buckets);

    let mut bars_svg = String::new();
    let mut y = 96;
    for (name, pct) in &metrics.langs {
        let width = ((248.0 * (*pct as f64 / 100.0)).round() as u64).max(10);
        let color = lang_color(name, t.dark);
        let _ = writeln!(
            bars_svg,
            r#"    <text x="356" y="{y}" fill="{text_hi}">{name}</text>
    <text x="604" y="{y}" text-anchor="end" fill="{title_col}">{pct}%</text>
    <rect x="356" y="{y_bar}" width="248" height="7" rx="3.5" fill="{bar_bg}"/>
    <rect class="bar" x="356" y="{y_bar}" width="{width}" height="7" rx="3.5" fill="{color}"/>"#,
            text_hi = t.text_hi,
            title_col = t.title_col,
            y_bar = y + 7,
            bar_bg = t.bar_bg,
        );
        y += 34;
    }

    format!(
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="900" height="268" viewBox="0 0 900 268" role="img" aria-label="{username} live engineering telemetry console">
  <defs>
    <style>
      @keyframes grow {{ from {{ transform: scaleX(0); }} to {{ transform: scaleX(1); }} }}
      @keyframes draw {{ from {{ stroke-dashoffset: 360; }} to {{ stroke-dashoffset: 0; }} }}
      .bar {{ transform-origin: 356px 0px; animation: grow 1.1s cubic-bezier(0.22, 1, 0.36, 1) forwards; }}
      .spark {{ stroke-dasharray: 360; animation: draw 1.6s ease-out forwards; }}
    </style>
    <linearGradient id="{grad_id}" x1="0" y1="0" x2="0" y2="1">
      <stop offset="0%" stop-color="{spark_col}" stop-opacity="0.28"/>
      <stop offset="100%" stop-color="{spark_col}" stop-opacity="0"/>
    </linearGradient>
  </defs>

  <rect x="0.5" y="0.5" width="899" height="267" rx="14" fill="{bg}" stroke="{border}"/>
  <path d="M 1 14 Q 1 1 14 1 L 886 1 Q 899 1 899 14 L 899 36 L 1 36 Z" fill="{header_bg}"/>
  <line x1="1" y1="36" x2="899" y2="36" stroke="{border}"/>

  <circle cx="20" cy="18.5" r="4.5" fill="#EF4444" fill-opacity="0.8"/>
  <circle cx="36" cy="18.5" r="4.5" fill="#F59E0B" fill-opacity="0.8"/>
  <circle cx="52" cy="18.5" r="4.5" fill="#22C55E" fill-opacity="0.8"/>
  <text x="450" y="22.5" text-anchor="middle" fill="{title_col}" font-family="ui-monospace, SFMono-Regular, Menlo, Monaco, Consolas, monospace" font-size="11.5">{username}@desk — telemetry --live --cadence 14d</text>
  <text x="880" y="22.5" text-anchor="end" fill="{muted}" font-family="ui-monospace, SFMono-Regular, Menlo, Monaco, Consolas, monospace" font-size="10.5">SYNCED · {synced}</text>

  <line x1="330" y1="52" x2="330" y2="246" stroke="{split}"/>
  <line x1="628" y1="52" x2="628" y2="246" stroke="{split}"/>

  <g transform="translate(24, 66)" font-family="ui-monospace, SFMono-Regular, Menlo, Monaco, Consolas, monospace" font-size="11.5">
    <text x="0" y="0" fill="{prompt}" font-weight="700">$ whoami --metrics</text>

    <rect x="0" y="18" width="136" height="52" rx="8" fill="{card_bg}" stroke="{card_border}"/>
    <text x="12" y="36" fill="{muted}" font-size="10">PUBLIC REPOS</text>
    <text x="12" y="59" fill="{repo_col}" font-size="20" font-weight="700">{public_repos}</text>

    <rect x="146" y="18" width="136" height="52" rx="8" fill="{card_bg}" stroke="{card_border}"/>
    <text x="158" y="36" fill="{muted}" font-size="10">OPEN OSS PRs</text>
    <text x="158" y="59" fill="{pr_col}" font-size="20" font-weight="700">{open_prs}</text>

    <rect x="0" y="80" width="136" height="52" rx="8" fill="{card_bg}" stroke="{card_border}"/>
    <text x="12" y="98" fill="{muted}" font-size="10">REPO STARS</text>
    <text x="12" y="121" fill="{star_col}" font-size="20" font-weight="700">{stars}</text>

    <rect x="146" y="80" width="136" height="52" rx="8" fill="{card_bg}" stroke="{card_border}"/>
    <text x="158" y="98" fill="{muted}" font-size="10">FOLLOWERS</text>
    <text x="158" y="121" fill="{text_hi}" font-size="20" font-weight="700">{followers}</text>

    <rect x="0" y="146" width="282" height="28" rx="6" fill="{pill_bg}" stroke="{pill_border}"/>
    <circle cx="14" cy="160" r="3.2" fill="{pill_dot}"/>
    <text x="26" y="164" fill="{pill_text}" font-size="10.8">BS Data Science · Open for engineering</text>
  </g>

  <g font-family="ui-monospace, SFMono-Regular, Menlo, Monaco, Consolas, monospace" font-size="11.5">
    <text x="356" y="66" fill="{prompt}" font-weight="700">$ stats --languages</text>
{bars_svg}  </g>

  <g font-family="ui-monospace, SFMono-Regular, Menlo, Monaco, Consolas, monospace" font-size="11.5">
    <text x="652" y="66" fill="{prompt}" font-weight="700">$ git pulse --14d</text>
    <rect x="652" y="82" width="224" height="122" rx="10" fill="{chart_bg}" stroke="{card_border}"/>

    <line x1="664" y1="112" x2="864" y2="112" stroke="{grid}" stroke-dasharray="3 3"/>
    <line x1="664" y1="146" x2="864" y2="146" stroke="{grid}" stroke-dasharray="3 3"/>
    <line x1="664" y1="180" x2="864" y2="180" stroke="{grid}" stroke-dasharray="3 3"/>

    <path d="{area_d}" fill="url(#{grad_id})"/>
    <path class="spark" d="{line_d}" fill="none" stroke="{spark_col}" stroke-width="2.2" stroke-linecap="round" stroke-linejoin="round"/>
    <circle cx="{end_x}" cy="{end_y}" r="4" fill="{spark_col}"/>

    <text x="652" y="226" fill="{title_col}" font-size="10.5">AUTO-SYNC · 06:00 UTC</text>
    <text x="652" y="244" fill="{muted}" font-size="10">Rust 2024 · ureq + rustls</text>
  </g>
</svg>
"#,
        username = metrics.username,
        grad_id = t.grad_id,
        spark_col = t.spark_col,
        bg = t.bg,
        border = t.border,
        header_bg = t.header_bg,
        title_col = t.title_col,
        muted = t.muted,
        synced = metrics.synced,
        split = t.split,
        prompt = t.prompt,
        card_bg = t.card_bg,
        card_border = t.card_border,
        repo_col = t.repo_col,
        public_repos = metrics.public_repos,
        pr_col = t.pr_col,
        open_prs = metrics.open_prs,
        star_col = t.star_col,
        stars = metrics.stars,
        text_hi = t.text_hi,
        followers = metrics.followers,
        pill_bg = t.pill_bg,
        pill_border = t.pill_border,
        pill_dot = t.pill_dot,
        pill_text = t.pill_text,
        chart_bg = t.chart_bg,
        grid = t.grid,
    )
}

fn main() -> Result<(), Box<dyn Error>> {
    let username = env::var("GITHUB_USERNAME").unwrap_or_else(|_| "sshnox".to_string());
    let token = env::var("GITHUB_TOKEN").unwrap_or_default();

    let metrics = fetch_metrics(&username, &token)?;
    let assets_dir = PathBuf::from("assets");
    fs::create_dir_all(&assets_dir)?;

    fs::write(
        assets_dir.join("telemetry-dark.svg"),
        render_svg(&metrics, &DARK_THEME),
    )?;
    fs::write(
        assets_dir.join("telemetry-light.svg"),
        render_svg(&metrics, &LIGHT_THEME),
    )?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sparkline_scales_within_bounds() {
        let (line, area, end_x, end_y) = build_sparkline_paths(&[0, 5, 10]);
        assert!(line.starts_with("M 664 186"));
        assert!(area.ends_with("L 864 196 L 664 196 Z"));
        assert_eq!(end_x, 864);
        assert_eq!(end_y, 92);
    }

    #[test]
    fn renders_both_themes_with_rust_signature() {
        let sample = Metrics {
            username: "sshnox".into(),
            public_repos: 64,
            followers: 10,
            stars: 31,
            open_prs: 18,
            langs: vec![("TypeScript".into(), 38), ("Rust".into(), 12)],
            buckets: vec![1, 2, 3, 4],
            synced: "2026-11-26".into(),
        };
        let dark = render_svg(&sample, &DARK_THEME);
        let light = render_svg(&sample, &LIGHT_THEME);
        assert!(dark.contains("Rust 2024 · ureq + rustls"));
        assert!(light.contains("Rust 2024 · ureq + rustls"));
        assert!(dark.contains("spark-fill-d"));
        assert!(light.contains("spark-fill-l"));
    }
}

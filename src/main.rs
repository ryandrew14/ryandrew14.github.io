use std::fs;
use std::path::Path;

use chrono::Datelike;
use pulldown_cmark::{html, Parser};
use serde::Serialize;
use tera::{Context, Tera};

#[derive(Debug, serde::Deserialize)]
struct FrontMatter {
    title: String,
    date: Option<String>,
    description: Option<String>,
}

#[derive(Debug, Serialize)]
struct Post {
    title: String,
    date: String,
    description: String,
    slug: String,
    content: String,
}

const CONTENT_DIR: &str = "content";
const TEMPLATES_DIR: &str = "templates";
const STATIC_DIR: &str = "static";
const OUTPUT_DIR: &str = "dist";

// -------- MAIN --------

fn main() {
    let serve = std::env::args().skip(1).any(|arg| arg == "serve");

    build();

    if serve {
        serve_dir(Path::new(OUTPUT_DIR), "127.0.0.1:8000");
    }
}

fn build() {
    let base_path = std::env::var("SITE_BASE_PATH").unwrap_or_else(|_| "/".to_string());
    let base_path = if base_path.ends_with('/') {
        base_path
    } else {
        format!("{base_path}/")
    };

    let output = Path::new(OUTPUT_DIR);
    if output.exists() {
        fs::remove_dir_all(output).expect("failed to clean output directory");
    }
    fs::create_dir_all(output).expect("failed to create output directory");

    copy_dir(Path::new(STATIC_DIR), &output.join("static"));

    let tera = Tera::new(&format!("{TEMPLATES_DIR}/**/*.html")).expect("failed to load templates");

    let year = chrono::Local::now().year();

    // Home / about page
    create_basic_page_from_markdown(&tera, year, "about.md", "index.html", &base_path, output);

    // Links page
    let links_dir = output.join("links");
    fs::create_dir_all(&links_dir).expect("failed to create links directory");
    create_basic_page_from_markdown(&tera, year, "links.md", "links.html", &base_path, &links_dir);

    // Blog posts
    let blog_posts_count = create_blog_page_from_markdown(&tera, year, &base_path, &output);
    println!("Built site into ./{OUTPUT_DIR} ({} blog posts)", blog_posts_count);
}

/// Serves the static files under `root` over HTTP until the process is killed.
fn serve_dir(root: &Path, addr: &str) {
    let server = tiny_http::Server::http(addr)
        .unwrap_or_else(|e| panic!("failed to start server on {addr}: {e}"));
    println!("Serving ./{OUTPUT_DIR} at http://{addr} (Ctrl-C to stop)");

    for request in server.incoming_requests() {
        // Strip query string and leading slash, default the root to index.html.
        let url = request.url().split('?').next().unwrap_or("/");
        let rel = url.trim_start_matches('/');
        let mut path = root.join(rel);
        if path.is_dir() {
            path = path.join("index.html");
        }

        // Guard against path traversal: the resolved path must stay under root.
        let canonical_root = fs::canonicalize(root).ok();
        let in_root = fs::canonicalize(&path)
            .ok()
            .zip(canonical_root)
            .is_some_and(|(p, base)| p.starts_with(base));

        let response = if in_root {
            match fs::read(&path) {
                Ok(bytes) => {
                    let content_type = content_type_for(&path);
                    let header = tiny_http::Header::from_bytes(&b"Content-Type"[..], content_type)
                        .expect("valid header");
                    tiny_http::Response::from_data(bytes).with_header(header)
                }
                Err(_) => tiny_http::Response::from_string("404 Not Found").with_status_code(404),
            }
        } else {
            tiny_http::Response::from_string("404 Not Found").with_status_code(404)
        };

        let _ = request.respond(response);
    }
}


// -------- HELPERS --------

fn content_type_for(path: &Path) -> &'static str {
    match path.extension().and_then(|e| e.to_str()) {
        Some("html") => "text/html; charset=utf-8",
        Some("css") => "text/css; charset=utf-8",
        Some("js") => "text/javascript; charset=utf-8",
        Some("json") => "application/json",
        Some("svg") => "image/svg+xml",
        Some("png") => "image/png",
        Some("jpg") | Some("jpeg") => "image/jpeg",
        Some("gif") => "image/gif",
        Some("webp") => "image/webp",
        Some("ico") => "image/x-icon",
        Some("woff2") => "font/woff2",
        Some("woff") => "font/woff",
        Some("txt") => "text/plain; charset=utf-8",
        _ => "application/octet-stream",
    }
}

/// Loads the markdown file at `markdown_filename` under `content`, renders it to HTML, and writes it to `output/index.html`.
fn create_basic_page_from_markdown(tera: &Tera, year: i32, markdown_filename: &str, template_filename: &str, base_path: &str, output: &Path) {
    let (front, html_content) = render_markdown(Path::new(CONTENT_DIR).join(markdown_filename));
    let mut ctx = Context::new();
    ctx.insert("title", &front.title);
    ctx.insert("description", &front.description);
    ctx.insert("content", &html_content);
    ctx.insert("base_path", base_path);
    ctx.insert("year", &year);
    write_page(tera, template_filename, &ctx, &output.join("index.html"));
}

/// Loads all the MD files in `content/blog`, renders them to HTML, and writes them to `output/blog`,
/// returning the number of blog posts processed
fn create_blog_page_from_markdown(tera: &Tera, year: i32, base_path: &str, output: &Path) -> usize {
    let mut posts = load_posts(Path::new(CONTENT_DIR).join("blog"));
    posts.sort_by(|a, b| b.date.cmp(&a.date));

    let blog_dir = output.join("blog");
    fs::create_dir_all(&blog_dir).expect("failed to create blog directory");

    let mut ctx = Context::new();
    ctx.insert("posts", &posts);
    ctx.insert("base_path", base_path);
    ctx.insert("year", &year);
    write_page(tera, "blog_index.html", &ctx, &blog_dir.join("index.html"));

    for post in &posts {
        let post_dir = blog_dir.join(&post.slug);
        fs::create_dir_all(&post_dir).expect("failed to create post directory");

        let mut ctx = Context::new();
        ctx.insert("post", post);
        ctx.insert("base_path", &base_path);
        ctx.insert("year", &year);
        write_page(&tera, "blog_post.html", &ctx, &post_dir.join("index.html"));
    }

    posts.len()
}

fn write_page(tera: &Tera, template: &str, ctx: &Context, dest: &Path) {
    let rendered = tera
        .render(template, ctx)
        .unwrap_or_else(|e| panic!("failed to render {template}: {e}"));
    fs::write(dest, rendered).unwrap_or_else(|e| panic!("failed to write {dest:?}: {e}"));
}

/// Splits a Markdown file with `+++`-delimited TOML front matter and renders
/// the remaining body to HTML.
fn render_markdown(path: impl AsRef<Path>) -> (FrontMatter, String) {
    let path = path.as_ref();
    let raw = fs::read_to_string(path).unwrap_or_else(|e| panic!("failed to read {path:?}: {e}"));

    let parts: Vec<&str> = raw.splitn(3, "+++").collect();
    if parts.len() != 3 {
        panic!("{path:?} is missing +++ front matter delimiters");
    }

    let front: FrontMatter = toml::from_str(parts[1].trim())
        .unwrap_or_else(|e| panic!("failed to parse front matter in {path:?}: {e}"));

    let parser = Parser::new(parts[2].trim());
    let mut html_out = String::new();
    html::push_html(&mut html_out, parser);

    (front, html_out)
}

fn load_posts(dir: impl AsRef<Path>) -> Vec<Post> {
    let dir = dir.as_ref();
    let mut posts = Vec::new();

    // I haven't written blog posts yet, this ensures fs::read_dir doesn't panic in this case
    // TODO remove when I write my first post
    if !dir.exists() {
        return posts;
    }

    let entries = fs::read_dir(dir).unwrap_or_else(|e| panic!("failed to read {dir:?}: {e}"));
    for entry in entries {
        let entry = entry.expect("failed to read directory entry");
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("md") {
            continue;
        }

        let slug = path
            .file_stem()
            .and_then(|s| s.to_str())
            .expect("post file has no stem")
            .to_string();

        let (front, content) = render_markdown(&path);
        posts.push(Post {
            title: front.title,
            date: front.date.unwrap_or_default(),
            description: front.description.unwrap_or_default(),
            slug,
            content,
        });
    }

    posts
}

/// Recursively copies `src` into `dst`, creating directories as needed.
fn copy_dir(src: &Path, dst: &Path) {
    if !src.exists() {
        return;
    }

    for entry in walkdir::WalkDir::new(src) {
        let entry = entry.expect("failed to walk static directory");
        let rel = entry
            .path()
            .strip_prefix(src)
            .expect("entry should be under src");
        let dest_path = dst.join(rel);

        if entry.file_type().is_dir() {
            fs::create_dir_all(&dest_path).expect("failed to create static directory");
        } else {
            fs::create_dir_all(dest_path.parent().unwrap())
                .expect("failed to create parent directory");
            fs::copy(entry.path(), &dest_path).expect("failed to copy static file");
        }
    }
}

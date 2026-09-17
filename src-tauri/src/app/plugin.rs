//! „Pobierz plugin”: zip w formacie pluginu Claude (`.claude-plugin/plugin.json` + `.mcp.json` + `skills/`),
//! generowany w chwili kliknięcia, bo `.mcp.json` wskazuje bezwzględną ścieżkę do TEGO zainstalowanego binarium.
//! Plik nie zawiera żadnych sekretów — tokeny zostają w credential store.

use std::io::Write;
use std::path::{Path, PathBuf};

use serde_json::json;

pub const PLUGIN_NAME: &str = "ecommerce-mcp";
pub const FILE_NAME: &str = "ecommerce-mcp-plugin.zip";

/// Treść wkompilowana w binarium: (ścieżka w zipie, zawartość).
const STATIC_FILES: [(&str, &str); 6] = [
    ("README.md", include_str!("../../plugin/README.md")),
    ("skills/przeglad-zamowien/SKILL.md", include_str!("../../plugin/skills/przeglad-zamowien/SKILL.md")),
    ("skills/obsluga-zamowienia/SKILL.md", include_str!("../../plugin/skills/obsluga-zamowienia/SKILL.md")),
    ("skills/produkty-i-stany/SKILL.md", include_str!("../../plugin/skills/produkty-i-stany/SKILL.md")),
    ("skills/raport-sprzedazy/SKILL.md", include_str!("../../plugin/skills/raport-sprzedazy/SKILL.md")),
    ("skills/allegro-sprzedaz/SKILL.md", include_str!("../../plugin/skills/allegro-sprzedaz/SKILL.md")),
];

pub fn build_zip(exe: &Path) -> Result<Vec<u8>, String> {
    let manifest = json!({
        "name": PLUGIN_NAME,
        "displayName": crate::APP_NAME,
        "version": crate::APP_VERSION,
        "description": "Praca z Twoim e-commerce (BaseLinker, Allegro) przez lokalny serwer E-commerce MCP: zamówienia, oferty, produkty, stany i raporty sprzedaży.",
        "author": { "name": "Lampartoms" },
        "keywords": ["e-commerce", "baselinker", "allegro", "zamówienia", "mcp"],
    });
    let mcp = json!({ "mcpServers": { PLUGIN_NAME: { "command": exe.display().to_string(), "args": [crate::MCP_ARG] } } });
    let pretty = |value: &serde_json::Value| serde_json::to_string_pretty(value).map_err(|e| e.to_string());

    let mut files = vec![(".claude-plugin/plugin.json".to_string(), pretty(&manifest)?), (".mcp.json".to_string(), pretty(&mcp)?)];
    files.extend(STATIC_FILES.iter().map(|(path, content)| (path.to_string(), content.to_string())));

    let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    let options = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated).unix_permissions(0o644);
    for (path, content) in files {
        zip.start_file(path, options).map_err(|e| e.to_string())?;
        zip.write_all(content.as_bytes()).map_err(|e| e.to_string())?;
    }
    Ok(zip.finish().map_err(|e| e.to_string())?.into_inner())
}

/// Zapisuje plugin do wskazanego katalogu (w aplikacji: Pobrane) i zwraca ścieżkę pliku.
pub fn export(exe: &Path, dir: &Path) -> Result<PathBuf, String> {
    let bytes = build_zip(exe)?;
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let path = dir.join(FILE_NAME);
    std::fs::write(&path, bytes).map_err(|e| format!("cannot write {}: {e}", path.display()))?;
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;

    fn read(archive: &mut zip::ZipArchive<std::io::Cursor<Vec<u8>>>, name: &str) -> String {
        let mut content = String::new();
        archive.by_name(name).unwrap_or_else(|_| panic!("brak {name} w zipie")).read_to_string(&mut content).unwrap();
        content
    }

    #[test]
    fn plugin_zip_has_manifest_mcp_server_and_valid_skills() {
        let exe = Path::new("/Applications/E-commerce MCP.app/Contents/MacOS/ecommerce-mcp");
        let tmp = tempfile::tempdir().unwrap();
        let path = export(exe, tmp.path()).unwrap();
        assert_eq!(path.file_name().unwrap(), FILE_NAME);
        let mut archive = zip::ZipArchive::new(std::io::Cursor::new(std::fs::read(path).unwrap())).unwrap();

        let manifest: serde_json::Value = serde_json::from_str(&read(&mut archive, ".claude-plugin/plugin.json")).unwrap();
        assert_eq!(manifest["name"], "ecommerce-mcp");
        assert_eq!(manifest["version"], crate::APP_VERSION);

        // serwer MCP = dołączone binarium w trybie `mcp`, bez npx/node i bez żadnych sekretów w env
        let mcp: serde_json::Value = serde_json::from_str(&read(&mut archive, ".mcp.json")).unwrap();
        let server = &mcp["mcpServers"]["ecommerce-mcp"];
        assert_eq!(server["command"], exe.display().to_string());
        assert_eq!(server["args"], json!(["mcp"]));
        assert!(server.get("env").is_none());

        let skills: Vec<String> = archive.file_names().filter(|n| n.ends_with("/SKILL.md")).map(String::from).collect();
        assert_eq!(skills.len(), 5, "{skills:?}");
        let registry = crate::integrations::Registry::default();
        let tools: Vec<&str> = registry.metas().iter().flat_map(|meta| registry.get(meta.id).unwrap().tools()).map(|t| t.name).collect();
        for skill in skills {
            let content = read(&mut archive, &skill);
            let dir = skill.split('/').nth(1).unwrap();
            let frontmatter = content.strip_prefix("---\n").and_then(|rest| rest.split_once("\n---\n")).map(|(fm, _)| fm).expect("frontmatter");
            assert!(frontmatter.contains(&format!("name: {dir}\n")), "{skill}: name musi odpowiadać katalogowi");
            assert!(frontmatter.contains("description: "), "{skill}");
            // skill nie może odsyłać do narzędzi, których serwer nie ma
            for word in content.split(|c: char| !(c.is_ascii_alphanumeric() || c == '_')) {
                let looks_like_tool = ["list_", "get_", "update_", "add_"].iter().any(|p| word.starts_with(p));
                assert!(!looks_like_tool || tools.contains(&word), "{skill}: nieznane narzędzie `{word}`");
            }
        }
    }
}

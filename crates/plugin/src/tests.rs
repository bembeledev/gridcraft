use gridcraft_core::{CellError, Value};
use serde_json::json;

use crate::values::{from_json, to_json};
use crate::{Manifest, parse_reply};

fn manifest(functions: serde_json::Value) -> String {
    json!({"id": "t.plugin", "name": "T", "run": {"command": "true"}, "functions": functions}).to_string()
}

#[test]
fn manifests_are_validated() {
    let m = Manifest::parse(&manifest(json!([{"name": "t.one", "maxArgs": 2}]))).unwrap();
    assert_eq!(m.functions[0].name, "T.ONE");
    assert_eq!((m.functions[0].min_args, m.functions[0].max_args), (0, 2));
    for bad in [
        json!([{"name": "1BAD"}]),
        json!([{"name": "BAD NAME"}]),
        json!([{"name": "T.X"}, {"name": "t.x"}]),
        json!([{"name": "T.X", "minArgs": 3, "maxArgs": 2}]),
        json!([{"name": "T.X", "maxArgs": 300}]),
    ] {
        assert!(Manifest::parse(&manifest(bad.clone())).is_err(), "{bad}");
    }
    assert!(Manifest::parse(r#"{"id": "bad id", "name": "x", "run": {"command": "x"}}"#).is_err());
    assert!(Manifest::parse(r#"{"id": "x", "name": "x", "run": {"command": ""}}"#).is_err());
    assert!(Manifest::parse("not json").is_err());
}

#[test]
fn one_manifest_serves_several_apps() {
    // WordCraft's text menu means nothing here; `uiFor.gridcraft` replaces `ui`.
    let both = json!({
        "id": "t.both", "name": "Both", "run": {"command": "true"}, "apps": ["wordcraft", "GridCraft"],
        "functions": [{"name": "T.TWICE", "minArgs": 1, "maxArgs": 1}],
        "commands": [{"id": "tidy", "label": "Tidy"}],
        "ui": {"menus": [{"menu": "text", "command": "tidy"}]},
        "uiFor": {"gridcraft": {"menus": [{"menu": "row", "command": "tidy"}]}, "wordcraft": {"menus": [{"menu": "picture", "command": "tidy"}]}}
    });
    let m = Manifest::parse(&both.to_string()).unwrap();
    assert_eq!(m.ui.menus.len(), 1);
    assert_eq!(m.ui.menus[0].menu, crate::MenuKind::Row);
    assert!(m.for_this_app());
    let only_word = json!({"id": "t.word", "name": "Word", "run": {"command": "true"}, "apps": ["wordcraft"]});
    assert!(Manifest::parse(&only_word.to_string()).unwrap_err().contains("not GridCraft"));
    // Without `uiFor.gridcraft`, `ui` is this app's: another app's menu names are an error.
    let plain = json!({"id": "t.p", "name": "P", "run": {"command": "true"}, "commands": [{"id": "go", "label": "Go"}], "ui": {"menus": [{"menu": "text", "command": "go"}]}});
    assert!(Manifest::parse(&plain.to_string()).is_err());
}

#[test]
fn ui_declarations_must_name_the_plugins_commands() {
    let with_ui = |ui: serde_json::Value| {
        json!({"id": "t.ui", "name": "T", "run": {"command": "true"},
            "commands": [{"id": "go", "label": "Go"}], "ui": ui})
        .to_string()
    };
    let m = Manifest::parse(&with_ui(json!({
        "ribbon": [{"tab": "Data", "group": "Mine", "buttons": [{"command": "go", "size": "small", "params": {"n": 1}}]}],
        "menus": [{"command": "go"}, {"menu": "sheetTab", "command": "go", "label": "Go here"}]
    })))
    .unwrap();
    assert_eq!(m.ui.ribbon[0].buttons[0].size, crate::ButtonSize::Small);
    assert_eq!(m.ui.menus[0].menu, crate::MenuKind::Cell);
    assert_eq!(m.ui.menus[1].menu, crate::MenuKind::SheetTab);
    assert_eq!(m.command("go").map(|c| c.label.as_str()), Some("Go"));
    for bad in [
        json!({"ribbon": [{"tab": "Data", "group": "G", "buttons": [{"command": "nope"}]}]}),
        json!({"ribbon": [{"tab": "Data", "group": "G", "buttons": []}]}),
        json!({"ribbon": [{"tab": " ", "group": "G", "buttons": [{"command": "go"}]}]}),
        json!({"ribbon": [{"tab": "Data", "group": "G", "buttons": [{"command": "go", "size": "huge"}]}]}),
        json!({"menus": [{"command": "nope"}]}),
        json!({"menus": [{"menu": "statusBar", "command": "go"}]}),
        json!({"menus": [{"command": "go", "label": "x".repeat(65)}]}),
    ] {
        assert!(Manifest::parse(&with_ui(bad.clone())).is_err(), "{bad}");
    }
}

#[test]
fn homepages_must_be_web_links() {
    let with = |url: &str| json!({"id": "t.home", "name": "T", "run": {"command": "true"}, "homepage": url}).to_string();
    assert_eq!(Manifest::parse(&with("https://example.org/doc")).unwrap().homepage, "https://example.org/doc");
    assert!(Manifest::parse(&with("")).is_ok());
    for bad in ["javascript:alert(1)", "file:///etc/passwd", "example.org", "https://a b"] {
        assert!(Manifest::parse(&with(bad)).is_err(), "{bad}");
    }
}

#[test]
fn turned_off_plugins_lose_their_functions_and_commands() {
    let m = Manifest::parse(
        &json!({"id": "t.toggle", "name": "T", "run": {"command": "./never-started"},
            "functions": [{"name": "TOGGLE.F"}], "commands": [{"id": "go", "label": "Go"}]})
        .to_string(),
    )
    .unwrap();
    crate::install(m, std::env::temp_dir()).unwrap();
    assert!(gridcraft_calc::is_known_function("TOGGLE.F"));
    assert_eq!(crate::set_enabled("t.toggle", false), Ok(true));
    assert_eq!(crate::set_enabled("t.toggle", false), Ok(false));
    assert!(!gridcraft_calc::is_known_function("TOGGLE.F"));
    assert_eq!(crate::get("t.toggle").map(|p| p.status()), Some("off".into()));
    assert!(crate::run_command("t.toggle", "go", json!({}), json!({})).unwrap_err().contains("turned off"));
    assert_eq!(crate::set_enabled("t.toggle", true), Ok(true));
    assert!(gridcraft_calc::is_known_function("TOGGLE.F"));
    // Off, its function name is free; another plugin taking it blocks turning it back on.
    crate::set_enabled("t.toggle", false).unwrap();
    let other =
        Manifest::parse(&json!({"id": "t.toggle2", "name": "T2", "run": {"command": "x"}, "functions": [{"name": "TOGGLE.F"}]}).to_string()).unwrap();
    crate::install(other, std::env::temp_dir()).unwrap();
    assert!(crate::set_enabled("t.toggle", true).is_err());
    assert!(crate::unload("t.toggle"));
    assert!(gridcraft_calc::is_known_function("TOGGLE.F"), "unloading the off plugin leaves the other's function");
    assert!(crate::unload("t.toggle2"));
    assert!(crate::set_enabled("t.toggle", true).is_err());
}

#[test]
fn built_in_function_names_are_refused() {
    let m = Manifest::parse(&manifest(json!([{"name": "SUM"}]))).unwrap();
    let e = crate::install(m, std::env::temp_dir()).err().unwrap();
    assert!(e.contains("built-in"), "{e}");
}

#[test]
fn values_round_trip() {
    let arr = from_json(&json!([[1, "a"], [true, {"error": "#N/A"}]]));
    let Value::Array(a) = &arr else { panic!("{arr:?}") };
    assert_eq!((a.rows, a.cols), (2, 2));
    assert_eq!(a.get(1, 1), Some(&Value::Error(CellError::NA)));
    assert_eq!(to_json(&arr), json!([[1.0, "a"], [true, {"error": "#N/A"}]]));
    assert_eq!(from_json(&json!(null)), Value::Empty);
    assert_eq!(from_json(&json!({"error": "#nope"})), Value::Error(CellError::Value));
    assert_eq!(from_json(&json!({"x": 1})), Value::Error(CellError::Value));
    assert_eq!(from_json(&json!([])), Value::Error(CellError::Calc));
    assert_eq!(from_json(&json!([[]])), Value::Error(CellError::Calc));
    assert_eq!(from_json(&json!([[[1]]])), from_json(&json!([[{"error": "#VALUE!"}]])));
    // A flat list is a row; ragged rows are padded with #N/A.
    let Value::Array(row) = from_json(&json!([1, 2, 3])) else { panic!() };
    assert_eq!((row.rows, row.cols), (1, 3));
    let Value::Array(r) = from_json(&json!([[1], [2, 3]])) else { panic!() };
    assert_eq!(r.get(0, 1), Some(&Value::Error(CellError::NA)));
    // Text is capped at Excel's 32,767 characters.
    let Value::Text(t) = from_json(&json!("é".repeat(40_000))) else { panic!() };
    assert_eq!(t.chars().count(), 32_767);
}

#[test]
fn command_replies_are_checked() {
    let r = parse_reply(json!({"commands": [{"command": "cell.set", "params": {"cell": "A1"}}], "message": "hi"})).unwrap();
    assert_eq!(r.commands.len(), 1);
    assert_eq!(r.message.as_deref(), Some("hi"));
    assert!(parse_reply(json!({"commands": {}})).is_err());
    assert!(parse_reply(json!({"commands": [{"params": {}}]})).is_err());
    assert!(parse_reply(json!(null)).unwrap().commands.is_empty());
}

mod install {
    use std::io::Write;

    use crate::install::{Repo, Version, parse_github_url, pick_asset, unpack};

    fn repo(owner: &str, name: &str, version: Version) -> Repo {
        Repo { owner: owner.into(), name: name.into(), version }
    }

    #[test]
    fn github_addresses_are_read() {
        for url in ["https://github.com/acme/fin-tools", "github.com/acme/fin-tools/", "http://www.github.com/acme/fin-tools.git", "acme/fin-tools"] {
            assert_eq!(parse_github_url(url), Ok(repo("acme", "fin-tools", Version::Latest)), "{url}");
        }
        assert_eq!(parse_github_url("https://github.com/acme/fin/releases/tag/v1.2.0"), Ok(repo("acme", "fin", Version::Release("v1.2.0".into()))));
        assert_eq!(parse_github_url("https://github.com/acme/fin/tree/feat/x"), Ok(repo("acme", "fin", Version::Tree("feat/x".into()))));
        assert_eq!(parse_github_url("https://github.com/acme/fin/releases/latest"), Ok(repo("acme", "fin", Version::Latest)));
        for bad in [
            "",
            "https://gitlab.com/acme/fin",
            "https://github.com/acme",
            "https://github.com/../fin",
            "https://github.com/acme/fin/blob/main/x",
            "https://github.com/acme/fin/tree/../../x",
            "ftp://github.com/acme/fin",
            "a/b/c",
        ] {
            assert!(parse_github_url(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn the_release_zip_for_this_platform_is_picked() {
        let names: Vec<String> = [
            "fin-1.0-linux-x86_64.zip",
            "fin-1.0-macos-x86_64.zip",
            "fin-1.0-macos-aarch64.zip",
            "fin-1.0-windows-x64.zip",
            "fin-1.0-macos-aarch64.tar.gz",
            "checksums.txt",
        ]
        .map(String::from)
        .to_vec();
        assert_eq!(pick_asset(&names, "macos", "aarch64"), Some(2));
        assert_eq!(pick_asset(&names, "macos", "x86_64"), Some(1));
        assert_eq!(pick_asset(&names, "windows", "x86_64"), Some(3));
        assert_eq!(pick_asset(&names, "linux", "aarch64"), None, "no build for it: install the source");
        let universal: Vec<String> = ["fin-darwin-universal.zip", "fin-linux-amd64.zip"].map(String::from).to_vec();
        assert_eq!(pick_asset(&universal, "macos", "aarch64"), Some(0));
        let any: Vec<String> = ["fin-plugin.zip", "fin-linux-arm64.zip"].map(String::from).to_vec();
        assert_eq!(pick_asset(&any, "macos", "aarch64"), Some(0), "a zip naming no platform suits every one");
        assert_eq!(pick_asset(&any, "linux", "aarch64"), Some(1), "this platform's build beats the generic zip");
        assert_eq!(pick_asset(&["darwin-tools-windows.zip".to_string()], "macos", "x86_64"), Some(0));
        assert_eq!(pick_asset(&[], "macos", "aarch64"), None);
    }

    fn zip(files: &[(&str, &[u8], u32)]) -> Vec<u8> {
        let mut out = std::io::Cursor::new(Vec::new());
        let mut w = zip::ZipWriter::new(&mut out);
        for (name, data, mode) in files {
            let opts = zip::write::SimpleFileOptions::default().unix_permissions(*mode);
            w.start_file(*name, opts).unwrap();
            w.write_all(data).unwrap();
        }
        w.finish().unwrap();
        out.into_inner()
    }

    fn manifest(id: &str) -> Vec<u8> {
        serde_json::json!({"id": id, "name": "Fin", "run": {"command": "./run.sh"}}).to_string().into_bytes()
    }

    #[test]
    fn a_source_zip_unpacks_into_a_folder_named_after_the_plugin() {
        let root = std::env::temp_dir().join(format!("gridcraft-unpack-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let m = manifest("acme.fin");
        let bytes = zip(&[
            ("acme-fin-1a2b3c/README.md", b"readme", 0o644),
            ("acme-fin-1a2b3c/plugin.json", &m, 0o644),
            ("acme-fin-1a2b3c/run.sh", b"#!/bin/sh\n", 0o755),
            ("acme-fin-1a2b3c/lib/util.py", b"x = 1\n", 0o644),
            ("acme-fin-1a2b3c/examples/other/plugin.json", b"{}", 0o644),
            ("elsewhere/secret.txt", b"not part of the plugin", 0o644),
        ]);
        let dir = unpack(&bytes, &root).unwrap();
        assert_eq!(dir, root.join("acme.fin"));
        assert!(dir.join("plugin.json").is_file() && dir.join("lib/util.py").is_file() && dir.join("examples/other/plugin.json").is_file());
        assert!(!dir.join("elsewhere").exists() && !root.join("elsewhere").exists());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(std::fs::metadata(dir.join("run.sh")).unwrap().permissions().mode() & 0o777, 0o755);
            assert_eq!(std::fs::metadata(dir.join("README.md")).unwrap().permissions().mode() & 0o777, 0o644);
        }
        crate::load(&dir).unwrap();
        assert!(crate::unload("acme.fin"));

        // Installing again replaces the earlier copy, leftovers included.
        std::fs::write(dir.join("stale.txt"), "old").unwrap();
        let again = unpack(&zip(&[("plugin.json", &m, 0o644), ("run.sh", b"new", 0o755)]), &root).unwrap();
        assert_eq!(again, dir);
        assert!(!dir.join("stale.txt").exists() && !dir.join("lib").exists());
        assert_eq!(std::fs::read_dir(&root).unwrap().count(), 1, "no staging folders left behind");
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn unsafe_or_foreign_zips_are_refused() {
        let root = std::env::temp_dir().join(format!("gridcraft-unpack-bad-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        assert!(unpack(b"not a zip", &root).unwrap_err().contains("not a zip"));
        assert!(unpack(&zip(&[("README.md", b"hi", 0o644)]), &root).unwrap_err().contains("plugin.json"));
        assert!(unpack(&zip(&[("plugin.json", &manifest(".."), 0o644)]), &root).is_err(), "an id can't climb out of the folder");
        assert!(unpack(&zip(&[("plugin.json", b"{", 0o644)]), &root).is_err());
        // Paths escaping the plugin folder are skipped, not written.
        let m = manifest("acme.safe");
        let dir = unpack(&zip(&[("plugin.json", &m, 0o644), ("../escaped.txt", b"x", 0o644), ("/abs.txt", b"x", 0o644)]), &root).unwrap();
        assert!(!root.join("escaped.txt").exists() && !std::env::temp_dir().join("escaped.txt").exists());
        assert!(dir.join("plugin.json").is_file());
        std::fs::remove_dir_all(&root).unwrap();
    }
}

mod office {
    use crate::office::{Action, OfficeCommand, Permission, Step, parse_manifest};
    use crate::{MenuKind, run_command};

    /// An add-in manifest written for these tests from the public schema: a default task pane, a
    /// button on the Home tab, a menu with a function command on a custom tab, and a cell menu item.
    pub(super) const MANIFEST: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<OfficeApp xmlns="http://schemas.microsoft.com/office/appforoffice/1.1" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance"
  xmlns:bt="http://schemas.microsoft.com/office/officeappbasictypes/1.0" xmlns:ov="http://schemas.microsoft.com/office/taskpaneappversionoverrides" xsi:type="TaskPaneApp">
  <Id>0d5c7a1e-1111-4bcd-9eaf-0123456789ab</Id>
  <Version>1.0.0.0</Version>
  <ProviderName>Acme &amp; Sons</ProviderName>
  <DefaultLocale>en-US</DefaultLocale>
  <DisplayName DefaultValue="Acme Prices"/>
  <Description DefaultValue="Live prices in your sheet."/>
  <IconUrl DefaultValue="https://acme.example/icon-32.png"/>
  <SupportUrl DefaultValue="https://acme.example/help"/>
  <AppDomains><AppDomain>https://login.acme.example</AppDomain></AppDomains>
  <Hosts><Host Name="Workbook"/></Hosts>
  <Requirements><Sets DefaultMinVersion="1.1"><Set Name="ExcelApi" MinVersion="1.3"/></Sets></Requirements>
  <DefaultSettings><SourceLocation DefaultValue="https://acme.example/taskpane.html"/></DefaultSettings>
  <Permissions>ReadWriteDocument</Permissions>
  <VersionOverrides xmlns="http://schemas.microsoft.com/office/taskpaneappversionoverrides" xsi:type="VersionOverridesV1_0">
    <Hosts>
      <Host xsi:type="Workbook">
        <DesktopFormFactor>
          <FunctionFile resid="Commands.Url"/>
          <ExtensionPoint xsi:type="PrimaryCommandSurface">
            <OfficeTab id="TabHome">
              <Group id="Acme.Group"><Label resid="Group.Label"/>
                <Control xsi:type="Button" id="Acme.Taskpane">
                  <Label resid="Button.Label"/>
                  <Supertip><Title resid="Button.Label"/><Description resid="Button.Tip"/></Supertip>
                  <Action xsi:type="ShowTaskpane"><TaskpaneId>Main</TaskpaneId><SourceLocation resid="Taskpane.Url"/></Action>
                </Control>
              </Group>
            </OfficeTab>
            <CustomTab id="Acme.Tab"><Label resid="Tab.Label"/>
              <Group id="Acme.Tools"><Label resid="Tools.Label"/>
                <Control xsi:type="Menu" id="Acme.Menu"><Label resid="Menu.Label"/>
                  <Items>
                    <Item id="Acme.Refresh"><Label resid="Refresh.Label"/><Action xsi:type="ExecuteFunction"><FunctionName>refreshAll</FunctionName></Action></Item>
                    <Item id="Acme.Bad"><Label resid="Refresh.Label"/><Action xsi:type="ShowTaskpane"><SourceLocation resid="Http.Url"/></Action></Item>
                  </Items>
                </Control>
              </Group>
            </CustomTab>
          </ExtensionPoint>
          <ExtensionPoint xsi:type="ContextMenu">
            <OfficeMenu id="ContextMenuCell">
              <Control xsi:type="Button" id="Acme.Cell"><Label resid="Cell.Label"/>
                <Action xsi:type="ShowTaskpane"><SourceLocation resid="Taskpane.Url"/></Action>
              </Control>
            </OfficeMenu>
          </ExtensionPoint>
        </DesktopFormFactor>
      </Host>
    </Hosts>
    <Resources>
      <bt:Urls>
        <bt:Url id="Taskpane.Url" DefaultValue="https://acme.example/taskpane.html"/>
        <bt:Url id="Commands.Url" DefaultValue="https://acme.example/commands.html"/>
        <bt:Url id="Http.Url" DefaultValue="http://acme.example/insecure.html"/>
      </bt:Urls>
      <bt:ShortStrings>
        <bt:String id="Group.Label" DefaultValue="Acme"/>
        <bt:String id="Button.Label" DefaultValue="Show Prices"/>
        <bt:String id="Tab.Label" DefaultValue="Acme"/>
        <bt:String id="Tools.Label" DefaultValue="Tools"/>
        <bt:String id="Menu.Label" DefaultValue="More"/>
        <bt:String id="Refresh.Label" DefaultValue="Refresh All"/>
        <bt:String id="Cell.Label" DefaultValue="Price This Cell"/>
      </bt:ShortStrings>
      <bt:LongStrings><bt:String id="Button.Tip" DefaultValue="Opens the prices pane."/></bt:LongStrings>
    </Resources>
  </VersionOverrides>
</OfficeApp>"#;

    #[test]
    fn office_manifests_become_plugins() {
        let m = parse_manifest(MANIFEST, "gridcraft").unwrap();
        assert_eq!(m.id, "office.0d5c7a1e-1111-4bcd-9eaf-0123456789ab");
        assert_eq!((m.name.as_str(), m.author.as_str(), m.homepage.as_str()), ("Acme Prices", "Acme & Sons", "https://acme.example/help"));
        assert_eq!(m.apps, ["gridcraft"]);
        let o = m.office.as_ref().unwrap();
        assert_eq!(o.permissions, Permission::ReadWriteDocument);
        assert_eq!(o.requirements, [("ExcelApi".to_string(), "1.3".to_string())]);
        assert_eq!(o.app_domains, ["https://login.acme.example"]);
        // The default pane, the Home button, the menu's function (the http:// item is dropped) and the cell item.
        let ids: Vec<&str> = m.commands.iter().map(|c| c.id.as_str()).collect();
        assert_eq!(ids, ["taskpane", "Acme.Taskpane", "Acme.Refresh", "Acme.Cell"]);
        assert_eq!(
            o.commands["Acme.Taskpane"],
            OfficeCommand::ShowTaskpane { url: "https://acme.example/taskpane.html".into(), taskpane_id: Some("Main".into()) }
        );
        assert_eq!(
            o.commands["Acme.Refresh"],
            OfficeCommand::ExecuteFunction {
                function: "refreshAll".into(),
                url: "https://acme.example/commands.html".into(),
                control: "Acme.Refresh".into()
            }
        );
        assert_eq!(m.ui.ribbon.len(), 2);
        assert_eq!((m.ui.ribbon[0].tab.as_str(), m.ui.ribbon[0].group.as_str()), ("Home", "Acme"));
        assert_eq!(m.ui.ribbon[0].buttons[0].tooltip.as_deref(), Some("Opens the prices pane."));
        assert_eq!((m.ui.ribbon[1].tab.as_str(), m.ui.ribbon[1].buttons[0].label.as_deref()), ("Acme", Some("Refresh All")));
        assert_eq!(m.ui.menus.len(), 1);
        assert_eq!(m.ui.menus[0].menu, MenuKind::Cell);
    }

    #[test]
    fn office_commands_open_panes_instead_of_processes() {
        let m = parse_manifest(MANIFEST, "gridcraft").unwrap();
        let id = m.id.clone();
        crate::install(m, std::env::temp_dir()).unwrap();
        let r = run_command(&id, "Acme.Cell", serde_json::json!({}), serde_json::json!({})).unwrap();
        assert!(matches!(r.office, Some(OfficeCommand::ShowTaskpane { .. })));
        assert!(r.commands.is_empty());
        assert_eq!(crate::get(&id).unwrap().status(), "ready", "no process was started");
        assert_eq!(crate::get(&id).unwrap().info()["kind"], "office");
        assert!(crate::unload(&id));
    }

    #[test]
    fn foreign_or_broken_office_manifests_are_refused() {
        assert!(parse_manifest(MANIFEST, "wordcraft").unwrap_err().contains("not this app"));
        for bad in [
            "<OfficeApp/>",
            "not xml <<<",
            "<Other/>",
            &MANIFEST.replace("0d5c7a1e-1111-4bcd-9eaf-0123456789ab", "../../etc"),
            &MANIFEST.replace("https://acme.example/", "http://acme.example/"),
        ] {
            assert!(parse_manifest(bad, "gridcraft").is_err(), "{bad:.40}");
        }
        let deep = format!("<OfficeApp>{}{}</OfficeApp>", "<a>".repeat(100), "</a>".repeat(100));
        assert!(parse_manifest(&deep, "gridcraft").unwrap_err().contains("deeply"));
    }

    #[test]
    fn unmet_requirement_sets_block_the_add_in() {
        let xml = MANIFEST.replace("0d5c7a1e-1111-4bcd-9eaf-0123456789ab", "0d5c7a1e-2222-4bcd-9eaf-0123456789ab").replace(
            r#"<Requirements><Sets DefaultMinVersion="1.1"><Set Name="ExcelApi" MinVersion="1.3"/></Sets></Requirements>"#,
            r#"<Requirements><Sets DefaultMinVersion="1.9"><Set Name="ExcelApi"/><Set Name="Selection" MinVersion="1.1"/></Sets>
               <Methods><Method Name="Document.getSelectedDataAsync"/><Method Name="Document.goToByIdAsync"/></Methods></Requirements>"#,
        );
        let m = parse_manifest(&xml, "gridcraft").unwrap();
        let o = m.office.as_ref().unwrap();
        assert_eq!(
            o.requirements,
            [("ExcelApi".to_string(), "1.9".to_string()), ("Selection".to_string(), "1.1".to_string())],
            "DefaultMinVersion applies"
        );
        assert_eq!(o.methods, ["Document.getSelectedDataAsync", "Document.goToByIdAsync"]);
        let id = m.id.clone();
        crate::install(m, std::env::temp_dir()).unwrap();
        let p = crate::get(&id).unwrap();
        assert_eq!(p.state(), "blocked");
        let why: Vec<String> = p.blocked_by().iter().map(|b| b.message.clone()).collect();
        assert_eq!(why, ["Needs ExcelApi 1.9; GridCraft supports up to 1.4", "Needs Document.goToByIdAsync, which GridCraft doesn't support"]);
        assert!(run_command(&id, "Acme.Cell", serde_json::json!({}), serde_json::json!({})).unwrap_err().contains("blocked"));
        assert!(crate::unload(&id));
    }

    #[test]
    fn sync_actions_read_as_json() {
        let a: Vec<Action> = serde_json::from_str(
            r#"[{"op":"load","path":[{"p":"workbook"},{"m":"getSelectedRange"}],"props":["values","address"]},
                {"op":"set","path":[{"p":"workbook"},{"p":"worksheets"},{"m":"getItem","a":["Data"]},{"m":"getRange","a":["A1:B2"]}],"prop":"values","value":[[1,2],[3,4]]},
                {"op":"call","path":[{"p":"workbook"},{"m":"getSelectedRange"}],"method":"clear","args":["Formats"]}]"#,
        )
        .unwrap();
        assert!(matches!(&a[1], Action::Set { prop, .. } if prop == "values"));
        assert!(matches!(&a[0], Action::Load { path, .. } if path[1] == Step::Method { m: "getSelectedRange".into(), a: vec![] }));
    }
}

// ---------------------------------------------------------------- protocol 1.2

mod protocol_1_2 {
    use serde_json::{Map, json};

    use crate::dialog::{Content, Dialog, FieldKind, OpenRequest};
    use crate::{Manifest, parse_reply};

    #[test]
    fn manifests_declare_events() {
        let m = Manifest::parse(
            &json!({"id": "t.ev", "name": "Ev", "run": {"command": "true"}, "events": ["documentSaved", "documentClosed", "somethingLater"]})
                .to_string(),
        )
        .unwrap();
        assert_eq!(m.events, ["documentSaved", "documentClosed", "somethingLater"], "names this host doesn't know are kept and ignored");
        let none = Manifest::parse(&json!({"id": "t.ev", "name": "Ev", "run": {"command": "true"}}).to_string()).unwrap();
        assert!(none.events.is_empty());
        assert!(Manifest::parse(&json!({"id": "t.ev", "name": "Ev", "run": {"command": "true"}, "events": "documentSaved"}).to_string()).is_err());
        assert!(crate::FEATURES.contains(&"dialogs") && crate::FEATURES.contains(&"events"));
    }

    #[test]
    fn list_dialogs_are_validated_and_truncated() {
        let items: Vec<_> = (0..10_050).map(|i| json!({"id": format!("/f/{i}"), "label": format!("File {i}"), "icon": "workbook"})).collect();
        let d = Dialog::parse(&json!({
            "title": "t".repeat(300),
            "kind": "list",
            "prompt": "p".repeat(5000),
            "items": items,
            "command": "browse",
            "params": {"account": "me"},
            "buttons": (0..6).map(|i| json!({"label": format!("B{i}"), "command": "browse", "params": {"n": i}})).collect::<Vec<_>>(),
        }))
        .unwrap();
        assert_eq!(d.title.chars().count(), 200);
        assert_eq!(d.prompt.chars().count(), 4000);
        assert_eq!(d.ok_label, "OK");
        assert_eq!(d.buttons.len(), 4);
        let Content::List { items, multi } = &d.content else { panic!("a list") };
        assert_eq!(items.len(), 10_000);
        assert!(!multi);
        assert_eq!(items[3].label, "File 3");
        // OK runs the command with the dialog's params plus the item.
        assert_eq!(d.ok_params(&["/f/3".into()], &Map::new()), json!({"account": "me", "id": "/f/3"}));
        // A button gets its own params, plus the selection when there is one.
        assert_eq!(d.button_params(1, &[], &Map::new()), Some(("browse".into(), json!({"n": 1}))));
        assert_eq!(d.button_params(1, &["/f/1".into()], &Map::new()), Some(("browse".into(), json!({"n": 1, "id": "/f/1"}))));
        assert_eq!(d.button_params(9, &[], &Map::new()), None);
        // Long texts are cut, ids at 4,096 characters.
        let long = Dialog::parse(&json!({"kind": "list", "command": "go", "multi": true,
            "items": [{"id": "i".repeat(5000), "label": "l".repeat(300), "detail": "d".repeat(300)}]}))
        .unwrap();
        let Content::List { items, multi } = &long.content else { panic!("a list") };
        assert!(multi);
        assert_eq!((items[0].id.len(), items[0].label.len(), items[0].detail.len()), (4096, 256, 256));
        assert_eq!(long.ok_params(&["a".into(), "b".into()], &Map::new()), json!({"ids": ["a", "b"]}));
        // The normalized form round-trips.
        assert_eq!(Dialog::parse(&d.to_json()).unwrap(), d);
    }

    #[test]
    fn form_and_message_dialogs() {
        let d = Dialog::parse(&json!({
            "title": "Sign in", "kind": "form", "command": "login", "okLabel": "Sign In",
            "fields": [
                {"id": "server", "label": "Server", "value": "https://cloud.example"},
                {"id": "password", "label": "Password", "kind": "secret"},
                {"id": "port", "label": "Port", "kind": "number", "value": "443"},
                {"id": "remember", "label": "Remember", "kind": "checkbox", "value": true},
                {"id": "mode", "label": "Mode", "kind": "choice", "value": "nope", "options": [{"id": "a", "label": "A"}, {"id": "b", "label": "B"}]}
            ]
        }))
        .unwrap();
        let Content::Form { fields } = &d.content else { panic!("a form") };
        assert_eq!(
            fields.iter().map(|f| f.kind).collect::<Vec<_>>(),
            [FieldKind::Text, FieldKind::Secret, FieldKind::Number, FieldKind::Checkbox, FieldKind::Choice]
        );
        assert_eq!(fields[1].value, json!(""));
        assert_eq!(fields[2].value, json!(443.0));
        assert_eq!(fields[4].value, json!("a"), "an unknown choice falls back to the first option");
        let mut values = Map::new();
        values.insert("server".into(), json!("x"));
        assert_eq!(d.ok_params(&[], &values), json!({"fields": {"server": "x"}}));
        let many: Vec<_> = (0..20).map(|i| json!({"id": format!("f{i}")})).collect();
        let Content::Form { fields } = Dialog::parse(&json!({"kind": "form", "command": "go", "fields": many})).unwrap().content else {
            panic!("a form")
        };
        assert_eq!(fields.len(), 16);

        // A message needs no command: OK just closes.
        let m = Dialog::parse(&json!({"kind": "message", "title": "Done", "prompt": "Uploaded.\nAll good."})).unwrap();
        assert_eq!(m.command, None);
        assert_eq!(m.commands().count(), 0);
    }

    #[test]
    fn malformed_dialogs_are_errors() {
        for bad in [
            json!("a dialog"),
            json!({"title": "No kind", "command": "go"}),
            json!({"kind": "web", "command": "go"}),
            json!({"kind": "list"}),
            json!({"kind": "list", "command": "go", "items": "nope"}),
            json!({"kind": "list", "command": "go", "items": [{"label": "no id"}]}),
            json!({"kind": "list", "command": "go", "items": [3]}),
            json!({"kind": "list", "command": "bad command", "items": []}),
            json!({"kind": "list", "command": "go", "params": [1, 2]}),
            json!({"kind": "list", "command": "go", "multi": "yes"}),
            json!({"kind": "form", "command": "go", "fields": [{"id": "x", "kind": "slider"}]}),
            json!({"kind": "form", "command": "go", "fields": [{"label": "no id"}]}),
            json!({"kind": "form", "command": "go", "fields": [{"id": "x", "kind": "checkbox", "value": "yes"}]}),
            json!({"kind": "message", "buttons": [{"label": "Retry"}]}),
            json!({"kind": "message", "buttons": [{"command": "go"}]}),
            json!({"kind": "message", "title": 7}),
        ] {
            assert!(Dialog::parse(&bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn open_requests_are_checked() {
        let o = OpenRequest::parse(&json!({"path": "/tmp/a.xlsx", "label": "x".repeat(300), "readOnly": true})).unwrap();
        assert_eq!((o.path.as_str(), o.label.as_ref().map(|l| l.chars().count()), o.read_only), ("/tmp/a.xlsx", Some(200), true));
        assert_eq!(OpenRequest::parse(&json!({"path": "/tmp/a.xlsx"})).unwrap().label, None);
        for bad in [
            json!("/tmp/a.xlsx"),
            json!({}),
            json!({"path": ""}),
            json!({"path": 3}),
            json!({"path": "/a\u{0}b.xlsx"}),
            json!({"path": "/a.xlsx", "readOnly": 1}),
        ] {
            assert!(OpenRequest::parse(&bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn replies_carry_open_and_dialog() {
        let r = parse_reply(json!({"open": {"path": "/tmp/b.xlsx"}, "dialog": {"kind": "message", "prompt": "hi"}, "message": "m"})).unwrap();
        assert_eq!(r.open.map(|o| o.path), Some("/tmp/b.xlsx".into()));
        assert!(matches!(r.dialog.map(|d| d.content), Some(Content::Message)));
        assert!(parse_reply(json!({"dialog": {"kind": "nope"}})).is_err());
        assert!(parse_reply(json!({"open": {"label": "no path"}})).is_err());
        let plain = parse_reply(json!({"open": null, "dialog": null})).unwrap();
        assert!(plain.open.is_none() && plain.dialog.is_none());
    }

    #[test]
    fn events_refuse_host_sync() {
        let e = crate::events::host_during_event("t.p", "documentSaved", "host/sync", &json!({"actions": []})).unwrap_err();
        assert_eq!(e.data["code"], "AccessDenied");
        assert!(crate::events::host_during_event("t.p", "documentSaved", "host/vbaProject/set", &json!({})).is_err());
        assert_eq!(crate::events::host_during_event("t.p", "documentSaved", "host/log", &json!({"text": "hi"})).unwrap(), json!({}));
        assert_eq!(crate::events::host_during_event("t.p", "documentSaved", "host/nope", &json!({})).unwrap_err().code, -32601);
        assert_eq!(crate::events::timeout("documentClosed").as_secs(), 5);
        assert_eq!(crate::events::timeout("documentSaved").as_secs(), 30);
    }

    #[test]
    fn the_host_language_is_a_tag() {
        crate::set_host_language("pt-BR; rm -rf /");
        assert_eq!(crate::host_language(), "pt-BRrm-rf");
        crate::set_host_language("it");
        assert_eq!(crate::host_language(), "it");
    }
}

// ---------------------------------------------------------------- requirements (manifest 1.3)

mod requirements {
    use serde_json::json;

    use crate::Manifest;

    fn add(id: &str, version: &str, extra: serde_json::Value) {
        let mut m = json!({"id": id, "name": id.rsplit('.').next().unwrap_or(id), "version": version, "run": {"command": "./not-started"}});
        if let (Some(m), Some(extra)) = (m.as_object_mut(), extra.as_object()) {
            m.extend(extra.clone());
        }
        crate::install(Manifest::parse(&m.to_string()).unwrap(), std::env::temp_dir()).unwrap();
    }

    fn messages(id: &str) -> Vec<String> {
        crate::get(id).unwrap().blocked_by().iter().map(|b| b.message.clone()).collect()
    }

    #[test]
    fn blocked_add_ins_lose_their_functions_until_what_they_need_arrives() {
        add(
            "t.req.reports",
            "1.4.0",
            json!({"functions": [{"name": "REQTEST.REPORT"}], "commands": [{"id": "go", "label": "Go"}],
                   "requires": {"addins": [{"id": "t.req.vba", "version": "^0.1.4", "github": "sirmmo/craft-vba", "why": "Runs the report macros"}]}}),
        );
        let reports = crate::get("t.req.reports").unwrap();
        assert_eq!(reports.state(), "blocked");
        assert!(!reports.active() && reports.enabled());
        assert_eq!(messages("t.req.reports"), ["Needs craft-vba, which isn't installed"]);
        assert!(!gridcraft_calc::is_known_function("REQTEST.REPORT"), "a blocked add-in's functions are absent");
        assert!(crate::run_command("t.req.reports", "go", json!({}), json!({})).unwrap_err().contains("blocked"));
        let info = reports.info();
        assert_eq!(info["state"], "blocked");
        assert_eq!(info["blockedBy"][0]["kind"], "addin");
        assert_eq!(info["blockedBy"][0]["id"], "t.req.vba");
        assert_eq!(info["requires"]["addins"][0]["github"], "sirmmo/craft-vba");
        assert!(reports.status().starts_with("blocked: "));
        let missing = crate::missing("t.req.reports");
        assert_eq!((missing.len(), missing[0].name.as_str(), missing[0].version.as_str()), (1, "craft-vba", "^0.1.4"));

        // Out of range: still blocked; an update out of range conflicts.
        add("t.req.vba", "0.1.3", json!({}));
        assert_eq!(messages("t.req.reports"), ["Needs vba ^0.1.4 (installed: 0.1.3)"]);
        let newer = Manifest::parse(&json!({"id": "t.req.vba", "name": "vba", "version": "0.2.0", "run": {"command": "x"}}).to_string()).unwrap();
        assert_eq!(crate::conflicts(&newer).len(), 1, "0.2.0 doesn't suit reports either");

        // In range: unblocked without a restart, its functions back.
        add("t.req.vba", "0.1.5", json!({}));
        assert!(reports.active(), "{:?}", messages("t.req.reports"));
        assert_eq!(reports.state(), "loaded");
        assert!(gridcraft_calc::is_known_function("REQTEST.REPORT"));
        assert_eq!(crate::required_by("t.req.vba"), ["t.req.reports"]);
        assert_eq!(crate::get("t.req.vba").unwrap().info()["requiredBy"], json!(["t.req.reports"]));
        let order = crate::start_order();
        let at = |id: &str| order.iter().position(|x| x == id).unwrap();
        assert!(at("t.req.vba") < at("t.req.reports"), "{order:?}");
        let c = crate::conflicts(&newer);
        assert_eq!(c.len(), 1);
        assert_eq!(c[0].message, "reports needs vba ^0.1.4; 0.2.0 doesn't match");
        let tree = crate::requirements("t.req.reports").unwrap();
        assert_eq!(tree["satisfied"], true);
        assert_eq!((tree["tree"][0]["have"].as_str(), tree["tree"][0]["state"].as_str()), (Some("0.1.5"), Some("loaded")));

        // Turning the required one off blocks the dependent; back on unblocks it.
        assert_eq!(crate::dependents("t.req.vba"), ["t.req.reports"]);
        crate::set_enabled("t.req.vba", false).unwrap();
        assert_eq!(messages("t.req.reports"), ["Needs vba, which is turned off"]);
        assert!(!gridcraft_calc::is_known_function("REQTEST.REPORT"));
        crate::set_enabled("t.req.vba", true).unwrap();
        assert!(reports.active());
        // A required add-in that fails for good blocks its dependents too.
        crate::get("t.req.vba").unwrap().fail("flooded the app");
        assert_eq!(messages("t.req.reports"), ["Needs vba, which has failed"]);
        crate::set_enabled("t.req.vba", false).unwrap();
        crate::set_enabled("t.req.vba", true).unwrap();
        assert!(reports.active(), "turning it off and on gives it a fresh start");

        // Unloading it blocks the dependent again.
        assert!(crate::unload("t.req.vba"));
        assert_eq!(messages("t.req.reports"), ["Needs craft-vba, which isn't installed"]);
        let tree = crate::requirements("t.req.reports").unwrap();
        assert_eq!((tree["satisfied"].as_bool(), tree["tree"][0]["state"].as_str()), (Some(false), Some("missing")));
        assert!(crate::unload("t.req.reports"));
        assert!(!gridcraft_calc::is_known_function("REQTEST.REPORT"));
    }

    #[test]
    fn app_and_protocol_requirements_are_checked_against_this_host() {
        add(
            "t.req.future",
            "1.0.0",
            json!({"requires": {"apps": {"gridcraft": ">=99.0"}, "protocol": ">=9.0", "features": ["dialogs", "telepathy"]}}),
        );
        let why = messages("t.req.future");
        assert_eq!(why.len(), 3, "{why:?}");
        assert!(why[0].starts_with("Needs GridCraft 99.0 or later (this is "), "{why:?}");
        assert_eq!(why[1], "Needs plugin protocol 9.0 or later (this GridCraft has 2.0)");
        assert_eq!(why[2], "Needs `telepathy`, which this GridCraft doesn't have");
        add(
            "t.req.today",
            "1.0.0",
            json!({"requires": {"apps": {"gridcraft": ">=0.1", "deckcraft": ">=99"}, "protocol": ">=1.2", "features": ["dialogs", "push"]}}),
        );
        assert!(crate::get("t.req.today").unwrap().active());
        assert!(crate::unload("t.req.future") && crate::unload("t.req.today"));
    }

    #[test]
    fn cycles_block_and_unloading_breaks_them() {
        add("t.req.cyc-a", "1.0.0", json!({"requires": {"addins": [{"id": "t.req.cyc-b"}]}}));
        add("t.req.cyc-b", "1.0.0", json!({"requires": {"addins": [{"id": "t.req.cyc-a"}]}}));
        assert_eq!(messages("t.req.cyc-a"), ["Requirement cycle: cyc-a → cyc-b → cyc-a"]);
        assert_eq!(crate::get("t.req.cyc-b").unwrap().blocked_by()[0].kind, crate::requires::Kind::Cycle);
        assert!(crate::unload("t.req.cyc-b"));
        assert_eq!(messages("t.req.cyc-a"), ["Needs t.req.cyc-b, which isn't installed"]);
        assert!(crate::unload("t.req.cyc-a"));
    }
}

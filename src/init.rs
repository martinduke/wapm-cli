//! logic to init a directory for use with wapm

use crate::abi::Abi;
use crate::data::manifest::MANIFEST_FILE_NAME;
use crate::data::manifest::{Command, Manifest, Module, Package};
use crate::util;

use dialoguer::{Confirmation, Input, Select};
use semver::Version;
use std::{
    any::Any,
    collections::HashMap,
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
};

const WASI_LAST_VERSION: &str = "0.0.0-unstable";

fn construct_template_manifest_from_data(username: Option<String>, package_name: String) -> String {
    let name_string = if let Some(un) = username {
        format!("{}/{}", un, package_name)
    } else {
        package_name
    };
    format!(
        r#"[package]
name = "{}"
version = "0.1.0"
description = ""
"#,
        name_string
    )
}

pub fn ask(prompt: &str, default: Option<String>) -> Result<Option<String>, std::io::Error> {
    let mut input = Input::<String>::new().with_prompt(prompt);
    let value = if let Some(v) = default {
        Input::<String>::new()
            .with_prompt(prompt)
            .default(v)
            .interact()?
    } else {
        Input::<String>::new()
            .with_prompt(prompt)
            .default("".to_owned())
            .interact()?
    };
    if value.is_empty() {
        return Ok(None);
    }
    return Ok(Some(value));
}

pub fn ask_until_valid<F, VR, Err>(
    prompt: &str,
    default: Option<String>,
    validator: F,
) -> Result<VR, std::io::Error>
where
    F: Fn(&str) -> Result<VR, Err>,
    Err: std::fmt::Display,
    VR: Any,
{
    loop {
        let input = ask(prompt, default.clone())?;
        let validated = validator(&input.unwrap_or("".to_owned()));
        match validated {
            Err(e) => {
                println!("{}", e);
            }
            Ok(v) => {
                return Ok(v);
            }
        }
    }
}

pub fn validate_wasm_source(source: &str) -> Result<PathBuf, String> {
    if source == "none" || source.ends_with(".wasm") {
        return Ok(PathBuf::from(source));
    }
    return Err("The module source path must have a .wasm extension".to_owned());
}

pub fn validate_commands(command_names: &str) -> Result<String, util::NameError> {
    if command_names == "" {
        return Ok(command_names.to_owned());
    }
    util::validate_name(command_names)
}

pub fn init(dir: PathBuf, force_yes: bool) -> Result<(), failure::Error> {
    let manifest_location = {
        let mut dir = dir.clone();
        dir.push(MANIFEST_FILE_NAME);
        dir
    };
    let mut manifest = if manifest_location.exists() {
        Manifest::find_in_directory(dir)?
    } else {
        Manifest {
            base_directory_path: dir.clone(),
            fs: None,
            package: Package {
                name: dir
                    .clone()
                    .as_path()
                    .file_name()
                    .unwrap()
                    .to_str()
                    .unwrap()
                    .to_owned(),
                description: "".to_owned(),
                version: Version::parse("1.0.0").unwrap(),
                repository: None,
                // author: None,
                license: Some("ISC".to_owned()),
                license_file: None,
                homepage: None,
                wasmer_extra_flags: None,
                readme: None,
                disable_command_rename: false,
            },
            dependencies: None,
            module: Some(vec![Module {
                name: "entry".to_owned(),
                source: "entry.wasm".to_owned().into(),
                abi: Abi::default(),
                interfaces: None,
            }]),
            command: None,
        }
    };

    if !force_yes {
        println!(
            "This utility will walk you through creating a wapm.toml file.
It only covers the most common items, and tries to guess sensible defaults.

Use `wapm add <pkg>` afterwards to add a package and
save it as a dependency in the wapm.toml file.

Press ^C at any time to quit."
        );
        manifest.package.name = ask_until_valid(
            "Package name",
            Some(manifest.package.name),
            util::validate_name,
        )?;
        manifest.package.version = ask_until_valid(
            "Version",
            Some(manifest.package.version.to_string()),
            Version::parse,
        )?;
        manifest.package.description =
            ask("Description", Some(manifest.package.description))?.unwrap_or("".to_owned());
        manifest.package.repository = ask("Repository", manifest.package.repository)?;
        // author = ask("Author", &author)?;
        manifest.package.license = Some(ask_until_valid(
            "License",
            manifest.package.license,
            util::validate_license,
        )?);
        // Let's reset the modules
        let mut all_modules: Vec<Module> = vec![];
        let mut all_commands: Vec<Command> = vec![];
        loop {
            let current_index = all_modules.len();
            println!("Enter the data for the Module ({})", current_index + 1);
            let mut module = if current_index == 0 {
                Module {
                    name: "entry".to_owned(),
                    source: PathBuf::from("entry.wasm"),
                    abi: Abi::default(),
                    interfaces: None,
                }
            } else {
                Module {
                    name: "".to_owned(),
                    source: PathBuf::from("none"),
                    abi: Abi::default(),
                    interfaces: None,
                }
            };
            module.source = ask_until_valid(
                " - Source (path)",
                Some(module.source.to_str().unwrap().to_owned()),
                validate_wasm_source,
            )?;
            if module.source.to_str().unwrap() == "none" {
                break;
            }
            // Let's try to guess the name based on the file path
            let default_module_name = Path::new(&module.source)
                .file_stem()
                .unwrap()
                .to_str()
                .unwrap()
                .to_owned();
            module.name = ask_until_valid(
                " - Name",
                Some(default_module_name.clone()),
                util::validate_name,
            )?;
            let (abi, interfaces): (Abi, Option<HashMap<String, String>>) = match Select::new()
                .with_prompt(" - ABI")
                .item("None")
                .item("WASI")
                .item("Emscripten")
                .default(0)
                .interact()?
            {
                1 => (
                    Abi::Wasi,
                    Some(
                        [("wasi".to_owned(), WASI_LAST_VERSION.to_owned())]
                            .iter()
                            .cloned()
                            .collect(),
                    ),
                ),
                2 => (Abi::Emscripten, None),
                0 | _ => (Abi::None, None),
            };
            module.abi = abi;
            module.interfaces = interfaces;
            // We ask for commands if it has an Abi
            if module.abi == Abi::Wasi || module.abi == Abi::Emscripten {
                let commands = ask_until_valid(
                    " - Commmands (space separated)",
                    Some(default_module_name.clone()),
                    validate_commands,
                )?;
                if !commands.is_empty() {
                    all_commands.push(Command {
                        name: commands,
                        module: module.name.clone(),
                        main_args: None,
                        package: None,
                    });
                }
            }
            all_modules.push(module);
        }
        manifest.module = if all_modules.is_empty() {
            None
        } else {
            Some(all_modules)
        };
        manifest.command = if all_commands.is_empty() {
            None
        } else {
            Some(all_commands)
        };
    }

    let print_text = if force_yes {
        "Wrote to"
    } else {
        "About to write to"
    };
    println!(
        "\n{} {}:\n\n{}\n",
        print_text,
        manifest.base_directory_path.to_str().unwrap(),
        manifest.to_string()?
    );

    if force_yes
        || Confirmation::new()
            .with_text("Is this OK? (yes)")
            .default(true)
            .interact()?
    {
        manifest.save()?;
        #[allow(unused_must_use)]
        {
            init_gitignore(manifest.base_directory_path);
        }
    } else {
        println!("Aborted.")
    }
    Ok(())
}

pub fn init_gitignore(mut dir: PathBuf) -> Result<(), failure::Error> {
    let gitignore = {
        dir.push(".gitignore");
        dir
    };

    let mut f = fs::OpenOptions::new()
        .create(false)
        .read(true)
        .append(true)
        .open(gitignore)?;
    let mut gitignore_str = String::new();
    f.read_to_string(&mut gitignore_str)?;

    // TODO: this doesn't understand gitignores at all, it just checks for an entry
    // use crate that can check if a directory is ignored or not
    for line in gitignore_str.lines() {
        if line.contains("wapm_packages") {
            return Ok(());
        }
    }

    f.write_all(b"\nwapm_packages")?;
    Ok(())
}

#[derive(Debug, Fail)]
pub enum InitError {
    #[fail(display = "Manifest file already exists in {:?}", dir)]
    ManifestAlreadyExists { dir: PathBuf },
}

use clap::{Parser, Subcommand};
use maplit::hashmap;
use std::fs::File;
use std::io::{BufWriter, Write, read_to_string};
use std::path::{Path, PathBuf};
use tiger_lib::FileKind;
use tiger_lib::block::Block;
use tiger_lib::fileset::FileEntry;
use tiger_lib::parse::ParserMemory;
use tiger_lib::pdxfile::PdxFile;

#[derive(Parser)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Parses the game's buildings files and produces ones
    /// that add the correct number of modded buildings
    Buildings {
        input_path: PathBuf,
        output_path: PathBuf,
    },

    /// Parses the game's states files and updates them with
    /// the new sets of resources
    States {
        input_path: PathBuf,
        output_path: PathBuf,
    },
}

const BOM_CHAR: char = '\u{feff}';

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    match &cli.command {
        Commands::Buildings {
            input_path,
            output_path,
        } => {
            if !input_path.is_dir() {
                anyhow::bail!("Input path must be a directory");
            }
            if !output_path.is_dir() {
                anyhow::bail!("Output path must be a directory");
            }

            for entry in std::fs::read_dir(input_path)?.filter_map(Result::ok) {
                let in_path = entry.path();
                let parser = ParserMemory::default();
                let file_entry =
                    FileEntry::new(in_path.clone(), FileKind::Vanilla, in_path.clone());
                let contents =
                    PdxFile::read(&file_entry, &parser).expect("No file contents parsed");

                let out_path = output_path.join(format!(
                    "ir_{}",
                    in_path.file_name().unwrap().to_str().unwrap()
                ));
                create_modded_buildings_file(&contents, &out_path)?;
            }
        }
        Commands::States {
            input_path,
            output_path,
        } => {
            if !input_path.is_dir() {
                anyhow::bail!("Input path must be a directory");
            }
            if !output_path.is_dir() {
                anyhow::bail!("Output path must be a directory");
            }

            for entry in std::fs::read_dir(input_path)?.filter_map(Result::ok) {
                let in_path = entry.path();
                let out_path = output_path.join(in_path.file_name().unwrap().to_str().unwrap());
                create_modded_states_file_replace(&in_path, &out_path)?;
            }
        }
    }

    Ok(())
}

fn create_modded_buildings_file(contents: &Block, out_path: &Path) -> anyhow::Result<()> {
    let building_ratios = hashmap! {
        "building_textile_mill" => (4, "building_tailoring_workshop"),
        "building_furniture_manufactory" => (4, "building_luxury_furniture_manufactory"),
        "building_glassworks" => (4, "building_pottery_mill"),
        "building_rye_farm" => (6, "building_fruit_orchard"),
        "building_wheat_farm" => (6, "building_fruit_orchard"),
        "building_rice_farm" => (6, "building_fruit_orchard"),
        "building_millet_farm" => (6, "building_fruit_orchard"),
        "building_maize_farm" => (6, "building_fruit_orchard"),
        "building_livestock_ranch" => (2, "building_wool_farm"),
        "building_food_industry" => (4, "building_distillery"),
    };

    let mut out_file = BufWriter::new(File::create(out_path)?);
    writeln!(out_file, "{}BUILDINGS={{", BOM_CHAR)?;

    let buildings = contents
        .get_field_block("BUILDINGS")
        .expect("Missing BUILDINGS field");
    for (state_name, state_block) in buildings.iter_assignments_and_definitions() {
        writeln!(out_file, "\t{} = {{", state_name.as_str())?;
        for (region_state_name, region_state_block) in state_block
            .expect_block()
            .unwrap()
            .iter_assignments_and_definitions()
        {
            writeln!(out_file, "\t\t{} = {{", region_state_name.as_str())?;
            for (token, building) in region_state_block
                .expect_block()
                .unwrap()
                .iter_assignments_and_definitions()
            {
                if token.as_str() != "create_building" {
                    continue;
                }

                // Check if this building is of a split type
                let building = building.expect_block().unwrap();
                let building_type = building.get_field_value("building").unwrap();
                if !building_ratios.contains_key(building_type.as_str()) {
                    continue;
                }

                // Check if this building has the minimum number of levels for splitting
                let add_ownership = building.get_field_block("add_ownership").unwrap();
                let add_ownership_building = add_ownership.get_field_blocks("building");
                let add_ownership_country = add_ownership.get_field_blocks("country");
                let mut original_owners = add_ownership_building
                    .iter()
                    .map(|block| {
                        hashmap! {
                            "type" => block.get_field_value("type").unwrap().to_string(),
                            "country" => block.get_field_value("country").unwrap().to_string(),
                            "levels" => block.get_field_value("levels").unwrap().to_string(),
                            "region" => block.get_field_value("region").unwrap().to_string(),
                        }
                    })
                    .chain(add_ownership_country.iter().map(|block| {
                        hashmap! {
                            "country" => block.get_field_value("country").unwrap().to_string(),
                            "levels" => block.get_field_value("levels").unwrap().to_string(),
                        }
                    }))
                    .collect::<Vec<_>>();
                let total_building_levels = original_owners
                    .iter()
                    .map(|owner| owner.get("levels").unwrap().parse::<u16>().unwrap())
                    .sum::<u16>();
                let &(ratio, modded_building) =
                    building_ratios.get(building_type.as_str()).unwrap();
                let modded_building_levels =
                    (total_building_levels as f32 / ratio as f32 - 0.1).round() as u16;
                if modded_building_levels == 0 {
                    continue;
                }

                // Split the building, using a weighted approach for assigning owners
                writeln!(
                    out_file,
                    "\t\t\tremove_building = {}",
                    building_type.as_str()
                )?;
                original_owners.sort_unstable_by_key(|owner| {
                    owner.get("levels").unwrap().parse::<u16>().unwrap()
                });
                original_owners.reverse();
                let level_percentages = original_owners
                    .iter()
                    .map(|owner| {
                        owner.get("levels").unwrap().parse::<u16>().unwrap() as f32
                            / total_building_levels as f32
                    })
                    .collect::<Vec<_>>();
                let mut modded_per_owner = level_percentages
                    .iter()
                    .map(|&p| (modded_building_levels as f32 * p).round() as u16)
                    .collect::<Vec<_>>();

                let mut modded_sum = modded_per_owner.iter().sum::<u16>();
                let mut i = 0;
                while modded_sum > modded_building_levels {
                    // Remove starting from the back
                    modded_per_owner[original_owners.len() - 1 - i] -= 1;
                    i = (i + 1) % original_owners.len();
                    modded_sum -= 1;
                }
                while modded_sum < modded_building_levels {
                    // Add starting from the front
                    modded_per_owner[i] += 1;
                    i = (i + 1) % original_owners.len();
                    modded_sum += 1;
                }
                if modded_sum != modded_building_levels {
                    anyhow::bail!("Incorrect number of modded building levels, fix the code");
                }

                // Create the basic building
                writeln!(out_file, "\t\t\tcreate_building = {{")?;
                writeln!(
                    out_file,
                    "\t\t\t\tbuilding = \"{}\"",
                    building_type.as_str()
                )?;
                writeln!(out_file, "\t\t\t\tadd_ownership = {{")?;
                for (i, owner) in original_owners.iter().enumerate() {
                    let owned_by_building = owner.contains_key("type");
                    if owned_by_building {
                        writeln!(out_file, "\t\t\t\t\tbuilding = {{")?;
                        writeln!(
                            out_file,
                            "\t\t\t\t\t\ttype = \"{}\"",
                            owner.get("type").unwrap()
                        )?;
                        writeln!(
                            out_file,
                            "\t\t\t\t\t\tcountry = \"{}\"",
                            owner.get("country").unwrap()
                        )?;
                        writeln!(
                            out_file,
                            "\t\t\t\t\t\tlevels = {}",
                            owner.get("levels").unwrap().parse::<u16>().unwrap()
                                - modded_per_owner[i]
                        )?;
                        writeln!(
                            out_file,
                            "\t\t\t\t\t\tregion = \"{}\"",
                            owner.get("region").unwrap()
                        )?;
                        writeln!(out_file, "\t\t\t\t\t}}")?;
                    } else {
                        writeln!(out_file, "\t\t\t\t\tcountry = {{")?;
                        writeln!(
                            out_file,
                            "\t\t\t\t\t\tcountry = \"{}\"",
                            owner.get("country").unwrap()
                        )?;
                        writeln!(
                            out_file,
                            "\t\t\t\t\t\tlevels = {}",
                            owner.get("levels").unwrap().parse::<u16>().unwrap()
                                - modded_per_owner[i]
                        )?;
                        writeln!(out_file, "\t\t\t\t\t}}")?;
                    }
                }
                writeln!(out_file, "\t\t\t\t}}")?;
                writeln!(out_file, "\t\t\t}}")?;

                // Create the modded building
                writeln!(out_file, "\t\t\tcreate_building = {{")?;
                writeln!(out_file, "\t\t\t\tbuilding = \"{}\"", modded_building)?;
                writeln!(out_file, "\t\t\t\tadd_ownership = {{")?;
                for (i, owner) in original_owners.iter().enumerate() {
                    if modded_per_owner[i] == 0 {
                        break;
                    }

                    let owned_by_building = owner.contains_key("type");
                    if owned_by_building {
                        let owner_type = owner.get("type").unwrap();
                        writeln!(out_file, "\t\t\t\t\tbuilding = {{")?;
                        writeln!(
                            out_file,
                            "\t\t\t\t\t\ttype = \"{}\"",
                            if owner_type == building_type.as_str() {
                                modded_building
                            } else {
                                owner_type
                            }
                        )?;
                        writeln!(
                            out_file,
                            "\t\t\t\t\t\tcountry = \"{}\"",
                            owner.get("country").unwrap()
                        )?;
                        writeln!(out_file, "\t\t\t\t\t\tlevels = {}", modded_per_owner[i])?;
                        writeln!(
                            out_file,
                            "\t\t\t\t\t\tregion = \"{}\"",
                            owner.get("region").unwrap()
                        )?;
                        writeln!(out_file, "\t\t\t\t\t}}")?;
                    } else {
                        writeln!(out_file, "\t\t\t\t\tcountry = {{")?;
                        writeln!(
                            out_file,
                            "\t\t\t\t\t\tcountry = \"{}\"",
                            owner.get("country").unwrap()
                        )?;
                        writeln!(out_file, "\t\t\t\t\t\tlevels = {}", modded_per_owner[i])?;
                        writeln!(out_file, "\t\t\t\t\t}}")?;
                    }
                }
                writeln!(out_file, "\t\t\t\t}}")?;
                writeln!(
                    out_file,
                    "\t\t\t\treserves = {}",
                    building.get_field_value("reserves").unwrap().as_str()
                )?;
                writeln!(out_file, "\t\t\t}}")?;
            }
            writeln!(out_file, "\t\t}}")?;
        }
        writeln!(out_file, "\t}}")?;
    }

    writeln!(out_file, "}}")?;
    out_file.flush()?;

    Ok(())
}

#[allow(dead_code)]
fn create_modded_states_file_inject(in_path: &Path, out_path: &Path) -> anyhow::Result<()> {
    const FARM_TYPES: &[&str] = &[
        "building_rice_farm",
        "building_wheat_farm",
        "building_maize_farm",
        "building_millet_farm",
        "building_rye_farm",
    ];

    if in_path
        .file_stem()
        .unwrap()
        .to_string_lossy()
        .contains("99_seas")
    {
        return Ok(());
    }

    let in_data = read_to_string(File::open(in_path)?)?;

    let mut out_file = BufWriter::new(File::create(out_path)?);
    write!(out_file, "{}", BOM_CHAR)?;

    let mut depth = 0;
    let mut in_state = false;
    for mut line in in_data.lines() {
        line = line.trim_start_matches(BOM_CHAR);

        if line.contains('{') {
            depth += 1;
        }
        if line.contains('}') {
            depth -= 1;
        }

        // Start state
        if line.starts_with("STATE_") {
            writeln!(out_file, "INJECT:{}", line)?;
            in_state = true;
            continue;
        }

        // End state
        if in_state && depth == 0 {
            writeln!(out_file, "}}")?;
            in_state = false;
            continue;
        }

        if line.contains("arable_resources") {
            let mut modified_line = line.to_string();
            if FARM_TYPES
                .iter()
                .any(|&farm_type| modified_line.contains(farm_type))
            {
                modified_line = modified_line.replace("}", "\"building_fruit_orchard\" }");
            }
            if modified_line.contains("building_livestock_ranch") {
                modified_line = modified_line.replace("}", "\"building_wool_farm\" }");
            }
            writeln!(out_file, "{}", modified_line)?;
        }
    }

    out_file.flush()?;

    Ok(())
}

fn create_modded_states_file_replace(in_path: &Path, out_path: &Path) -> anyhow::Result<()> {
    const FARM_TYPES: &[&str] = &[
        "building_rice_farm",
        "building_wheat_farm",
        "building_maize_farm",
        "building_millet_farm",
        "building_rye_farm",
    ];

    if in_path
        .file_stem()
        .unwrap()
        .to_string_lossy()
        .contains("99_seas")
    {
        return Ok(());
    }

    let in_data = read_to_string(File::open(in_path)?)?;

    let mut out_file = BufWriter::new(File::create(out_path)?);
    write!(out_file, "{}", BOM_CHAR)?;

    for mut line in in_data.lines() {
        line = line.trim_start_matches(BOM_CHAR);
        if line.trim().starts_with("arable_resources") {
            let mut modified_line = line.to_string();
            if FARM_TYPES
                .iter()
                .any(|&farm_type| modified_line.contains(farm_type))
            {
                modified_line = modified_line.replace("}", "\"bg_fruit_orchard\" }");
            }
            if modified_line.contains("bg_livestock_ranches") {
                modified_line = modified_line.replace("}", "\"bg_wool_farm\" }");
            }
            writeln!(out_file, "{}", modified_line)?;
        } else {
            writeln!(out_file, "{}", line)?;
        }
    }

    out_file.flush()?;

    Ok(())
}

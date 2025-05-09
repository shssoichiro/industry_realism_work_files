use clap::{Parser, Subcommand};
use maplit::hashmap;
use std::fs::File;
use std::io::{BufWriter, Write};
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
    }

    Ok(())
}

fn create_modded_buildings_file(contents: &Block, out_path: &Path) -> anyhow::Result<()> {
    let building_ratios = hashmap! {
        "building_textile_mills" => (4, "building_tailoring_workshops"),
        "building_furniture_manufacturies" => (4, "building_luxury_furniture_manufacturies"),
        "building_glassworks" => (4, "building_pottery_mills"),
        "building_rye_farm" => (6, "building_fruit_orchard"),
        "building_wheat_farm" => (6, "building_fruit_orchard"),
        "building_rice_farm" => (6, "building_fruit_orchard"),
        "building_millet_farm" => (6, "building_fruit_orchard"),
        "building_maize_farm" => (6, "building_fruit_orchard"),
        "building_livestock_ranch" => (2, "building_wool_farms"),
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

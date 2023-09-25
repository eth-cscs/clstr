use clap::{arg, value_parser, ArgAction, ArgGroup, Command};

use strum::IntoEnumIterator;

use std::path::PathBuf;

use super::commands::get_nodes_artifacts;

pub fn subcommand_get_artifacts_node(hsm_group: Option<&String>) -> Command {

    let mut artifact_subcommand = 
            Command::new("artifacts")
                .aliases(["a", "art"])
                .about("Get node's artifacts")
                .arg_required_else_help(true)
                .arg(arg!(<XNAME> "xname").required(true))
                .arg(arg!(-t --type <TYPE> "Filters output to specific type").value_parser(get_nodes_artifacts::ArtifactType::iter().map(|e| e.into()).collect::<Vec<&str>>()))
                .arg(arg!(-o --output <FORMAT> "Output format. If missing it will print output data in human redeable (tabular) format").value_parser(["json"]))
                ;

    match hsm_group {
        None => {
            artifact_subcommand  = artifact_subcommand.arg(arg!(<HSM_GROUP_NAME> "hsm group name"))
        }
        Some(_) => {}
    }

    let get_node_artifacts = Command::new("nodes")
        .aliases(["n", "node", "nd"])
        .about("Get node's artifacts")
        .subcommand(
            artifact_subcommand
        );

    get_node_artifacts
}

pub fn subcommand_get_artifacts_hsm_group(hsm_group: Option<&String>) -> Command {
    let mut artifact_subcommand = Command::new("artifacts").aliases(["a", "art"]).about("Get HSM group's artifacts").arg(arg!(-o --output <FORMAT> "Output format. If missing it will print output data in human redeable (tabular) format").value_parser(["json"]));

    match hsm_group {
        None => {
            artifact_subcommand = artifact_subcommand
                .arg_required_else_help(true)
                .arg(arg!(<HSM_GROUP_NAME> "hsm group name"))
        }
        Some(_) => {
             artifact_subcommand = artifact_subcommand.arg_required_else_help(false);
        }
    }

    let get_hsm_group_artifacts = Command::new("hsm-groups")
        .aliases(["h", "hg", "hsm", "hsmgrops"])
        .about("Get HSM group's artifacts")
        .subcommand(artifact_subcommand)
        ;

    get_hsm_group_artifacts
}

pub fn subcommand_apply_hsm() -> Command {
    Command::new("hsm-group")
        .aliases(["hsm"])
        .arg_required_else_help(true)
        .about("Rearange nodes in a HSM group based on pattern")
        .arg(arg!(-p --pattern <VALUE> ... "Pattern to express the new HSM layout like `<hsm_group_name>[:<property>]*:<num_nodes>`. Where hsm_group_name (mandatory) is the target HSM group, property (optional) is the property (eg NVIDIA, A100, AMD, EPYC, etc) to filter nodes' components (Nodes[].Processors[].PopulatedFRU.ProcessorFRUInfo.Model or Nodes[].NodeAccels[].PopulatedFRU.NodeAccelFRUInfo.Model) and num_nodes (mandatory) is the number of nodes with those properties we need for the new HSM layout. Eg test:nvidia:a100:2 means `test` HSM group should have 2 nodes with NVIDIA A100, test:nvidia:2:amd:rome:3 means `test` HSM group will have 2 nvidia nodes and 3 AMD ROME nodes. NOTE: a single pattern may match multiple nodes therefore the total combination of num_nodes for a single HSM group does not accumulate.").required(true))
}

pub fn build_cli(hsm_group: Option<&String>) -> Command {
    Command::new("manta")
        .arg_required_else_help(true)
        .subcommand(
            Command::new("get")
                .alias("g")
                .arg_required_else_help(true)
                .about("Get cluster details")
                .subcommand(subcommand_get_artifacts_node(hsm_group))
                .subcommand(subcommand_get_artifacts_hsm_group(hsm_group)),
        )
        .subcommand(
            Command::new("apply")
                .alias("a")
                .arg_required_else_help(true)
                .about("Create new cluster")
                // .subcommand(subcommand_apply_cluster(/* hsm_group */))
                .subcommand(subcommand_apply_hsm(/* hsm_group */))
        )
        /* .subcommand(
            Command::new("update")
                .alias("u")
                .arg_required_else_help(true)
                .about("Update existing cluster details")
                .subcommand(subcommand_update_nodes(hsm_group))
                .subcommand(subcommand_update_hsm_group(hsm_group)),
        ) */
}

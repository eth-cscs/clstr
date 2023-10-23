use clap::ArgMatches;

use super::commands::{
    apply_hsm_based_on_component_quantity, get_hsm_artifacts, get_nodes_artifacts,
};

pub async fn process_cli(
    cli_apply: ArgMatches,
    shasta_token: &str,
    shasta_base_url: &str,
    shasta_root_cert: &[u8],
    hsm_group: Option<&String>,
) -> core::result::Result<(), Box<dyn std::error::Error>> {
    if let Some(cli_get) = cli_apply.subcommand_matches("get") {
        if let Some(cli_get_node) = cli_get.subcommand_matches("nodes") {
            if let Some(cli_get_node_artifacts) = cli_get_node.subcommand_matches("artifacts") {
                // Check HSM group name provided and configuration file
                let hsm_group_name = match hsm_group {
                    None => cli_get_node_artifacts.get_one::<String>("HSM_GROUP_NAME"),
                    Some(_) => hsm_group,
                };
                get_nodes_artifacts::exec(
                    shasta_token,
                    shasta_base_url,
                    shasta_root_cert,
                    hsm_group_name,
                    cli_get_node_artifacts.get_one::<String>("XNAME").unwrap(),
                    cli_get_node_artifacts.get_one::<String>("type"),
                    cli_get_node_artifacts.get_one::<String>("output"),
                )
                .await;
            }
        } else if let Some(cli_get_hsm_groups) = cli_get.subcommand_matches("hsm-groups") {
            if let Some(cli_get_hsm_groups_artifacts) =
                cli_get_hsm_groups.subcommand_matches("artifacts")
            {
                let hsm_group_name = match hsm_group {
                    None => cli_get_hsm_groups_artifacts
                        .get_one::<String>("HSM_GROUP_NAME")
                        .unwrap(),
                    Some(hsm_group_name_value) => hsm_group_name_value,
                };
                get_hsm_artifacts::exec(
                    shasta_token,
                    shasta_base_url,
                    shasta_root_cert,
                    hsm_group_name,
                    cli_get_hsm_groups_artifacts.get_one::<String>("output"),
                )
                .await;
            }
        }
    } else if let Some(cli_apply) = cli_apply.subcommand_matches("apply") {
        if let Some(cli_apply_hsm) = cli_apply.subcommand_matches("hsm-group") {
            apply_hsm_based_on_component_quantity::exec(
                shasta_token,
                shasta_base_url,
                shasta_root_cert,
                cli_apply_hsm.get_one::<String>("pattern").unwrap(),
                "nodes_free",
            )
            .await;
        }
    } /* else if let Some(cli_update) = cli_apply.subcommand_matches("update") {
          if let Some(cli_update_node) = cli_update.subcommand_matches("nodes") {
              let hsm_group_name = if hsm_group.is_none() {
                  cli_update_node.get_one::<String>("HSM_GROUP_NAME")
              } else {
                  hsm_group
              };
              update_node::exec(
                  shasta_token,
                  shasta_base_url,
                  hsm_group_name,
                  cli_update_node.get_one::<String>("boot-image"),
                  cli_update_node.get_one::<String>("desired-configuration"),
                  cli_update_node
                      .get_one::<String>("XNAMES")
                      .unwrap()
                      .split(',')
                      .map(|xname| xname.trim())
                      .collect(),
              )
              .await;
          } else if let Some(cli_update_hsm_group) = cli_update.subcommand_matches("hsm-group") {
              let hsm_group_name = if hsm_group.is_none() {
                  cli_update_hsm_group.get_one::<String>("HSM_GROUP_NAME")
              } else {
                  hsm_group
              };
              update_hsm_group::exec(
                  shasta_token,
                  shasta_base_url,
                  cli_update_hsm_group.get_one::<String>("boot-image"),
                  cli_update_hsm_group.get_one::<String>("desired-configuration"),
                  hsm_group_name.unwrap(),
              )
              .await;
          }
      } */

    Ok(())
}

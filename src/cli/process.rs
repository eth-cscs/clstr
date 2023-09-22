use std::io::IsTerminal;

use clap::ArgMatches;
use k8s_openapi::chrono;

use super::commands::{
    apply_cluster, apply_image, apply_node_off, apply_node_on, apply_node_reset, apply_session,
    apply_virt_env, console_cfs_session_image_target_ansible, console_node, get_configuration,
    get_hsm, get_images, get_nodes, get_session, get_template, log, update_hsm_group, update_node, apply_hsm_based_on_component_quantity
};

pub async fn process_cli(
    cli_apply: ArgMatches,
    shasta_token: &str,
    shasta_base_url: &str,
    vault_base_url: &str,
    vault_secret_path: &str,
    vault_role_id: &str,
    gitea_token: &str,
    gitea_base_url: &str,
    hsm_group: Option<&String>,
    // base_image_id: &str,
    k8s_api_url: &str,
) -> core::result::Result<(), Box<dyn std::error::Error>> {
    if let Some(cli_get) = cli_apply.subcommand_matches("get") {
        if let Some(cli_get_node) = cli_get.subcommand_matches("nodes") {
            // Check HSM group name provided and configuration file
            let hsm_group_name = match hsm_group {
                None => cli_get_node.get_one::<String>("HSM_GROUP_NAME"),
                Some(_) => hsm_group,
            };
            get_nodes::exec(
                shasta_token,
                shasta_base_url,
                hsm_group_name,
                *cli_get_node
                    .get_one::<bool>("nids-only-one-line")
                    .unwrap_or(&false),
                *cli_get_node
                    .get_one::<bool>("xnames-only-one-line")
                    .unwrap_or(&false),
                cli_get_node.get_one::<String>("output"),
            )
            .await;
        } else if let Some(cli_get_hsm_groups) = cli_get.subcommand_matches("hsm-groups") {
            let hsm_group_name = match hsm_group {
                None => cli_get_hsm_groups
                    .get_one::<String>("HSM_GROUP_NAME")
                    .unwrap(),
                Some(hsm_group_name_value) => hsm_group_name_value,
            };
            get_hsm::exec(shasta_token, shasta_base_url, hsm_group_name).await;
        }
    } else if let Some(cli_apply) = cli_apply.subcommand_matches("apply") {
        if let Some(cli_apply_hsm) = cli_apply.subcommand_matches("hsm-group") {
            apply_hsm_based_on_component_quantity::exec(
                shasta_token,
                shasta_base_url,
                cli_apply_hsm.get_one::<String>("pattern").unwrap(),
                "nodes_free",
            )
            .await;
        } else if let Some(cli_apply_node) = cli_apply.subcommand_matches("node") {
            if let Some(cli_apply_node_on) = cli_apply_node.subcommand_matches("on") {
                apply_node_on::exec(
                    hsm_group,
                    shasta_token,
                    shasta_base_url,
                    cli_apply_node_on
                        .get_one::<String>("XNAMES")
                        .unwrap()
                        .split(',')
                        .map(|xname| xname.trim())
                        .collect(),
                    cli_apply_node_on.get_one::<String>("reason").cloned(),
                )
                .await;
            } else if let Some(cli_apply_node_off) = cli_apply_node.subcommand_matches("off") {
                apply_node_off::exec(
                    hsm_group,
                    shasta_token,
                    shasta_base_url,
                    cli_apply_node_off
                        .get_one::<String>("XNAMES")
                        .unwrap()
                        .split(',')
                        .map(|xname| xname.trim())
                        .collect(),
                    cli_apply_node_off.get_one::<String>("reason").cloned(),
                    *cli_apply_node_off.get_one::<bool>("force").unwrap(),
                )
                .await;
            } else if let Some(cli_apply_node_reset) = cli_apply_node.subcommand_matches("reset") {
                apply_node_reset::exec(
                    hsm_group,
                    shasta_token,
                    shasta_base_url,
                    cli_apply_node_reset
                        .get_one::<String>("XNAMES")
                        .unwrap()
                        .split(',')
                        .map(|xname| xname.trim())
                        .collect(),
                    cli_apply_node_reset.get_one::<String>("reason"),
                    *cli_apply_node_reset
                        .get_one::<bool>("force")
                        .unwrap_or(&false),
                )
                .await;
            }
        }
    } else if let Some(cli_update) = cli_apply.subcommand_matches("update") {
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
    }

    Ok(())
}

use std::{sync::Arc, time::Instant};

use mesa::shasta::hsm;
use tokio::sync::Semaphore;

use crate::cli::commands::get_nodes_artifacts::{self, NodeSummary};

pub async fn exec(
    shasta_token: &str,
    shasta_base_url: &str,
    shasta_root_cert: &[u8],
    hsm_group_name: &str,
    output_opt: Option<&String>,
) {
    // Target HSM group
    let hsm_group_value = hsm::http_client::get_hsm_group(
        shasta_token,
        shasta_base_url,
        shasta_root_cert,
        hsm_group_name,
    )
    .await
    .unwrap();

    log::info!(
        "Get HW artifacts for nodes in HSM group '{:?}' and members {:?}",
        hsm_group_value["label"],
        hsm_group_value["members"]
    );

    // Get target HSM group members
    let hsm_group_target_members =
        hsm::utils::get_member_vec_from_hsm_group_value(&hsm_group_value);

    let mut node_summary_vec = Vec::new();

    let start_total = Instant::now();

    /* // Get HW inventory details for target HSM group
    for hsm_member in hsm_group_target_members.clone() {
        log::info!("Getting HW inventory details for node '{}'", hsm_member);

        let mut node_hw_inventory =
            hsm::http_client::get_hw_inventory(&shasta_token, &shasta_base_url, &hsm_member)
                .await
                .unwrap();

        node_hw_inventory = node_hw_inventory.pointer("/Nodes/0").unwrap().clone();
        let node_summary = NodeSummary::from_csm_value(node_hw_inventory.clone());
        node_summary_vec.push(node_summary);
    } */

    let mut tasks = tokio::task::JoinSet::new();

    let sem = Arc::new(Semaphore::new(5)); // CSM 1.3.1 higher number of concurrent tasks won't
                                           // make it faster

    // Get HW inventory details for target HSM group
    for hsm_member in hsm_group_target_members.clone() {
        let shasta_token_string = shasta_token.to_string(); // TODO: make it static
        let shasta_base_url_string = shasta_base_url.to_string(); // TODO: make it static
        let shasta_root_cert_vec = shasta_root_cert.to_vec();
        let hsm_member_string = hsm_member.to_string(); // TODO: make it static
                                                        //
        let permit = Arc::clone(&sem).acquire_owned().await;

        log::info!("Getting HW inventory details for node '{}'", hsm_member);
        tasks.spawn(async move {
            let _permit = permit; // Wait semaphore to allow new tasks https://github.com/tokio-rs/tokio/discussions/2648#discussioncomment-34885
            hsm::http_client::get_hw_inventory(
                &shasta_token_string,
                &shasta_base_url_string,
                &shasta_root_cert_vec,
                &hsm_member_string,
            )
            .await
            .unwrap()
        });
    }

    while let Some(message) = tasks.join_next().await {
        if let Ok(mut node_hw_inventory) = message {
            node_hw_inventory = node_hw_inventory.pointer("/Nodes/0").unwrap().clone();
            let node_summary = NodeSummary::from_csm_value(node_hw_inventory.clone());
            node_summary_vec.push(node_summary);
        } else {
            log::error!("Failed procesing/fetching node hw information");
        }
    }

    let duration = start_total.elapsed();

    if output_opt.is_some() && output_opt.unwrap().eq("json") {
        for node_summary in node_summary_vec {
            println!("{}", serde_json::to_string_pretty(&node_summary).unwrap());
        }
    } else {
        for node_summary in node_summary_vec {
            get_nodes_artifacts::print_table(&[node_summary].to_vec());
        }
    }

    log::info!(
        "Time elapsed in http calls to get hw inventory for HSM '{}' is: {:?}",
        hsm_group_name,
        duration
    );
}

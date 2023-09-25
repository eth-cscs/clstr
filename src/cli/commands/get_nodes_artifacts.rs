use comfy_table::{Cell, Table};
use mesa::shasta::hsm;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::str::FromStr;
use std::string::ToString;
use strum_macros::{AsRefStr, Display, EnumIter, EnumString, IntoStaticStr};
use termion::color;

#[derive(
    Debug, EnumIter, EnumString, IntoStaticStr, AsRefStr, Display, Serialize, Deserialize, Clone,
)]
pub enum ArtifactType {
    Memory,
    Processor,
    NodeAccel,
    NodeHsnNic,
    Drive,
    CabinetPDU,
    CabinetPDUPowerConnector,
    CMMRectifier,
    NodeAccelRiser,
    NodeEnclosurePowerSupplie,
    NodeBMC,
    RouterBMC,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct NodeSummary {
    xname: String,
    r#type: String,
    processors: Vec<ArtifactSummary>,
    memory: Vec<ArtifactSummary>,
    node_accels: Vec<ArtifactSummary>,
    node_hsn_nics: Vec<ArtifactSummary>,
}

impl NodeSummary {
    pub fn from_csm_value(hw_artifact_value: Value) -> Self {
        let processors = hw_artifact_value["Processors"]
            .as_array()
            .unwrap_or(&Vec::new())
            .iter()
            .map(|processor_value| ArtifactSummary::from_processor_value(processor_value.clone()))
            .collect();

        let memory = hw_artifact_value["Memory"]
            .as_array()
            .unwrap_or(&Vec::new())
            .iter()
            .map(|memory_value| ArtifactSummary::from_memory_value(memory_value.clone()))
            .collect();

        let node_accels = hw_artifact_value["NodeAccels"]
            .as_array()
            .unwrap_or(&Vec::new())
            .iter()
            .map(|nodeaccel_value| ArtifactSummary::from_nodeaccel_value(nodeaccel_value.clone()))
            .collect();

        let node_hsn_nics = hw_artifact_value["NodeHsnNics"]
            .as_array()
            .unwrap_or(&Vec::new())
            .iter()
            .map(|nodehsnnic_value| {
                ArtifactSummary::from_nodehsnnics_value(nodehsnnic_value.clone())
            })
            .collect();

        Self {
            xname: hw_artifact_value["ID"].as_str().unwrap().to_string(),
            r#type: hw_artifact_value["Type"].as_str().unwrap().to_string(),
            processors,
            memory,
            node_accels,
            node_hsn_nics,
        }
    }
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ArtifactSummary {
    xname: String,
    r#type: ArtifactType,
    info: String,
}

impl ArtifactSummary {
    fn from_processor_value(processor_value: Value) -> Self {
        Self {
            xname: processor_value["ID"].as_str().unwrap().to_string(),
            r#type: ArtifactType::from_str(processor_value["Type"].as_str().unwrap()).unwrap(),
            info: processor_value
                .pointer("/PopulatedFRU/ProcessorFRUInfo/Model")
                .unwrap()
                .as_str()
                .unwrap()
                .to_string(),
        }
    }

    fn from_memory_value(memory_value: Value) -> Self {
        Self {
            xname: memory_value["ID"].as_str().unwrap().to_string(),
            r#type: ArtifactType::from_str(memory_value["Type"].as_str().unwrap()).unwrap(),
            info: memory_value
                .pointer("/PopulatedFRU/MemoryFRUInfo/CapacityMiB")
                .unwrap()
                .as_number()
                .unwrap()
                .to_string()
                + " MiB",
        }
    }

    fn from_nodehsnnics_value(nodehsnnic_value: Value) -> Self {
        Self {
            xname: nodehsnnic_value["ID"].as_str().unwrap().to_string(),
            r#type: ArtifactType::from_str(nodehsnnic_value["Type"].as_str().unwrap()).unwrap(),
            info: nodehsnnic_value
                .pointer("/NodeHsnNicLocationInfo/Description")
                .unwrap()
                .as_str()
                .unwrap()
                .to_string(),
        }
    }

    fn from_nodeaccel_value(nodeaccel_value: Value) -> Self {
        Self {
            xname: nodeaccel_value["ID"].as_str().unwrap().to_string(),
            r#type: ArtifactType::from_str(nodeaccel_value["Type"].as_str().unwrap()).unwrap(),
            info: "-- TODO --".to_string(),
        }
    }
}

/// Get nodes status/configuration for some nodes filtered by a HSM group.
pub async fn exec(
    shasta_token: &str,
    shasta_base_url: &str,
    hsm_group_name: Option<&String>,
    xname: &str,
    type_artifact_opt: Option<&String>,
    output_opt: Option<&String>,
) {
    let hsm_groups_resp =
        hsm::http_client::get_hsm_groups(shasta_token, shasta_base_url, hsm_group_name).await;

    let hsm_group_list = if hsm_groups_resp.is_err() {
        eprintln!(
            "No HSM group {}{}{} found!",
            color::Fg(color::Red),
            hsm_group_name.unwrap(),
            color::Fg(color::Reset)
        );
        std::process::exit(0);
    } else {
        hsm_groups_resp.unwrap()
    };

    if hsm_group_list.is_empty() {
        println!("No HSM group found");
        std::process::exit(0);
    }

    // Take all nodes for all hsm_groups found and put them in a Vec
    let mut hsm_groups_node_list: Vec<String> =
        hsm::utils::get_members_from_hsm_groups_serde_value(&hsm_group_list)
            .into_iter()
            .collect();

    hsm_groups_node_list.sort();

    let mut node_hw_inventory =
        &hsm::http_client::get_hw_inventory(&shasta_token, &shasta_base_url, xname)
            .await
            .unwrap();

    node_hw_inventory = &node_hw_inventory.pointer("/Nodes/0").unwrap();

    if let Some(type_artifact) = type_artifact_opt {
        node_hw_inventory = &node_hw_inventory
            .as_array()
            .unwrap()
            .iter()
            .find(|&node| node["ID"].as_str().unwrap().eq(xname))
            .unwrap()[type_artifact];
    }

    let node_summary = NodeSummary::from_csm_value(node_hw_inventory.clone());

    if output_opt.is_some() && output_opt.unwrap().eq("json") {
        println!("{}", serde_json::to_string_pretty(&node_summary).unwrap());
    } else {
        print_table(&vec![node_summary].to_vec());
    }
}

pub fn print_table(node_summary_vec: &Vec<NodeSummary>) {
    let mut table = Table::new();

    table.set_header(vec![
        "Node XName",
        "Component XName",
        "Component Type",
        "Component Info",
    ]);

    for node_summary in node_summary_vec {
        for processor in &node_summary.processors {
            table.add_row(vec![
                Cell::new(node_summary.xname.clone()),
                Cell::new(processor.xname.clone()),
                Cell::new(processor.r#type.clone()),
                Cell::new(processor.info.clone()),
            ]);
        }

        for memory in &node_summary.memory {
            table.add_row(vec![
                Cell::new(node_summary.xname.clone()),
                Cell::new(memory.xname.clone()),
                Cell::new(memory.r#type.clone()),
                Cell::new(memory.info.clone()),
            ]);
        }

        for node_accel in &node_summary.node_accels {
            table.add_row(vec![
                Cell::new(node_summary.xname.clone()),
                Cell::new(node_accel.xname.clone()),
                Cell::new(node_accel.r#type.clone()),
                Cell::new(node_accel.info.clone()),
            ]);
        }

        for node_hsn_nic in &node_summary.node_hsn_nics {
            table.add_row(vec![
                Cell::new(node_summary.xname.clone()),
                Cell::new(node_hsn_nic.xname.clone()),
                Cell::new(node_hsn_nic.r#type.clone()),
                Cell::new(node_hsn_nic.info.clone()),
            ]);
        }
    }

    println!("{table}");
}

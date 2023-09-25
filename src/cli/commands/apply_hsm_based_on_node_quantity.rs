use std::{collections::HashMap, time::Instant, sync::Arc};

use tokio::sync::Semaphore;

use crate::{
    cli::commands::apply_hsm_based_on_node_quantity::utils::hsm_node_hw_profile,
    shasta::hsm,
};

// TEST --> a hsm -p zinal:a100:epyc:a100:2:epyc:instinct:8:epyc:5
//
/// Updates HSM groups following a string pattern like <hsm group name>:<node type>:<quantity>:<node type>:<quantity>:...,<hsm group name>:... or <hsm group>:<node type>*:<quantity> and <node type> being <prop>:<prop>:... or <prop>(<prop>)*
/// Example: zinal:a100:epyc:2:epyc:3:epyc:instinct:1 will provide a list of nodes for HSM group
/// zinal with x2 nodes which contains epic AND a100, x3 nodes with epyc, x1 node with epyc AND instinct.
///
/// Convention:
/// a100 - hw component
/// a100:epyc - hw profile
/// a100:epyc:5 - hw profile counter
///
/// NOTE: pattern > hw profile > hw property. pattern --> zinal:a100:epyc:2:epyc:instinct:8:epyc:25,
/// hw profile --> a100:epyc or epyc:instinct, hw property --> a100 or epyc or instinct
///
/// OPTION: nodes needs to be geographically nearby, we meassure this by calculating the "distance" between nodes.
/// The distance between 2 nodes is represented by a synbolic number which can be calculated by comparing the xnames of the nodes (which does not need increase/scale linearly, as shown in the examples below)
/// xXcCsSbB -- distance 0 (same blade)
/// xXcCsS ---- distance 1 (same slot)
/// xXcC ------ distance 2 (same chasis)
/// xX -------- distance 3 (same rack)
/// ----------- distance 4 (different rack)
///
/// OPTION 2: We need an option to say if we want to "keep" the nodes already in the HSM group which complains with the user/dessired pattern or if
///we want to "reallocate all nodes" because we want them to be geographically next to each other.
///
/// OPTION 3: We need an option to submit the changes to the CSM API to create the new HSM groups
///
/// EDGE CASES:
/// same pattern that identifies same hw type duplicated in same hsm group (eg
/// zinal:a100:epyc:2:epyc:instinct:1:epyc:a100:1) where node type a100:epyc is mentioned twice,
/// which one should we take? for now we will give an error
///
/// multiple HSM groups (eg zinal:a100:3 hohgant:epyc:4) and how should we express this? should we
/// separate each HSM group by ',' or ' ', something else?
///
/// duplicated HSM group in input (eg zinal:a100:epyc:4,zinal:a100:epyc1:instinct:epyc:1) what to
/// do here? we group both or we use the last one only or we use the first one only? for now we
/// will give an error
///
/// if HSM group looses all its members, then ask user if HSM should be deleted

#[derive(Clone, Debug)]
pub struct HsmHwPatternSummary {
    user_defined_hw_profile_vec: Vec<String>,
    user_defined_hw_profile_vec_hw_prop_vec_sorted: Vec<Vec<String>>, // index 'i' must match the index
    // in node_counters[].1[i]
    node_counter_vec: Vec<(String, Vec<u8>)>,
}

impl HsmHwPatternSummary {
    pub fn get_hw_profile_counters_total_count(&self, hw_profile: Vec<String>) -> u8 {
        // Get hw_profile index in user_defined_hw_profile_vec_hw_prop_vec_sorted related to
        // hw_profile value
        let hw_profile_index = self
            .user_defined_hw_profile_vec_hw_prop_vec_sorted
            .iter()
            .position(|x| x.eq(&hw_profile))
            .unwrap();
        let total_count = self
            .node_counter_vec
            .iter()
            .map(|(_, counters)| counters[hw_profile_index])
            .sum();

        total_count
    }

    pub fn get_hw_profile_counters(&self) -> Vec<Vec<u8>> {
        let mut counters = Vec::new();

        for node_counter in &self.node_counter_vec {
            counters.push(node_counter.1.clone());
        }

        counters
    }

    /// Create or increments the hs profile counters for a node based on a list of hw properties
    pub fn insert_node_hw_properties_counter(
        &mut self,
        node: String,
        node_hw_property_list: Vec<String>,
        new_counter: u8,
    ) {
        /* println!(
            "\nAdding hw_property_list {:?} to node {}:",
            node_hw_property_list, node
        ); */
        // Get hw_profile index in user_defined_hw_profile_vec_hw_prop_vec_sorted related to
        // hw_profile value
        let mut counter = self
            .node_counter_vec
            .iter()
            .find(|(n, _)| n.eq(&node))
            .and_then(|(_, counters)| Some(counters))
            .unwrap_or(&vec![
                0u8;
                self.user_defined_hw_profile_vec_hw_prop_vec_sorted
                    .len()
            ])
            .clone();

        for (i, user_hw_profile) in self
            .user_defined_hw_profile_vec_hw_prop_vec_sorted
            .iter()
            .enumerate()
        {
            /* println!(
                "Comparing user hw profile {:?} with node hw properties {:?}",
                user_hw_profile, node_hw_property_list
            ); */
            if user_hw_profile
                .iter()
                .all(|user_hw_property| node_hw_property_list.contains(user_hw_property))
            {
                counter[i] += new_counter;
                /* println!(
                    "updating/incrementing index {} by {} --> {:?}",
                    i, new_counter, counter
                ); */
            }
        }

        self.node_counter_vec.push((node, counter.clone()));
    }

    /// Create or increments the hs profile counters for a node based on a list of hw properties
    pub fn insert_node_hw_profile_counter(
        &mut self,
        node: String,
        node_hw_property_list: Vec<String>,
        new_counter: u8,
    ) {
        /* println!(
            "\nAdding hw_property_list {:?} to node {}:",
            node_hw_property_list, node
        ); */
        // Get hw_profile index in user_defined_hw_profile_vec_hw_prop_vec_sorted related to
        // hw_profile value
        let mut counter = self
            .node_counter_vec
            .iter()
            .find(|(n, _)| n.eq(&node))
            .and_then(|(_, counters)| Some(counters))
            .unwrap_or(&vec![
                0u8;
                self.user_defined_hw_profile_vec_hw_prop_vec_sorted
                    .len()
            ])
            .clone();

        for (i, user_hw_profile) in self
            .user_defined_hw_profile_vec_hw_prop_vec_sorted
            .iter()
            .enumerate()
        {
            /* println!(
                "Comparing user hw profile {:?} with node hw properties {:?}",
                user_hw_profile, node_hw_property_list
            ); */
            if user_hw_profile.eq(&node_hw_property_list) {
                counter[i] += new_counter;
                /* println!(
                    "updating/incrementing index {} by {} --> {:?}",
                    i, new_counter, counter
                ); */
            }
        }

        self.node_counter_vec.push((node, counter.clone()));
    }

    pub fn get_hw_profile_total_counters(&self) -> Vec<(String, u16)> {
        let mut total_counters = Vec::new();
        for (index, user_defined_hw_profile) in self
            .user_defined_hw_profile_vec_hw_prop_vec_sorted
            .iter()
            .enumerate()
        {
            let mut total_counter = 0;
            for counter in &self.node_counter_vec {
                total_counter += counter.1[index];
            }
            total_counters.push((user_defined_hw_profile.join(":"), total_counter as u16));
        }

        total_counters
    }

    /// Removes x amount of nodes with a specific hw profile and returns them
    pub fn get_candidate_nodes_with_specific_hw_profile(
        &self,
        hw_profile: &String,
        num_candidate_nodes: u8,
    ) -> Vec<String> {
        let hw_profile_index: u8 = self
            .user_defined_hw_profile_vec_hw_prop_vec_sorted
            .iter()
            .position(|x| x.join(":").eq(hw_profile))
            .unwrap() as u8;
        let mut elems_to_remove: Vec<String> = self
            .node_counter_vec
            .iter()
            .filter(|node_counter| {
                node_counter
                    .1
                    .get(hw_profile_index as usize)
                    .unwrap()
                    .clone()
                    > 0
            })
            .map(|node_counter| node_counter.0.clone())
            .collect();

        if (elems_to_remove.len() as u8) < num_candidate_nodes {
            Vec::new()
        } else {
            elems_to_remove.sort();

            elems_to_remove[0..num_candidate_nodes as usize].to_vec()
        }
    }
}

pub async fn exec(
    _vault_base_url: &str,
    _vault_token: &str,
    shasta_token: &str,
    shasta_base_url: &str,
    pattern: &str,
    hsm_group_parent: &str,
) {
    // Normalize text in lowercase and separate each HSM group hw inventory pattern
    let pattern_lowercase = pattern.to_lowercase();

    let mut pattern_element_vec: Vec<&str> = pattern_lowercase.split(':').collect();

    let target_hsm_group_name = pattern_element_vec.remove(0);

    // println!("hsm group: {}", hsm_group_name);
    // println!("pattern: {:?}", pattern_element_vec);

    // let mut dessired_hsm_node_hw_profile_vec = Vec::new();

    let mut pattern_node_type_vec: Vec<String> = Vec::new();
    let mut user_defined_hw_properties_grouped_by_hw_profile_vec_sorted: Vec<Vec<String>> =
        Vec::new();
    let mut user_defined_hw_profile_qty_hashmap: HashMap<String, u8> = HashMap::new();
    let mut user_defined_hw_profile_target_hsm_members_hashmap: HashMap<String, Vec<String>> =
        HashMap::new();
    let mut user_defined_hw_profile_hsm_free_node_members_hashmap: HashMap<String, Vec<String>> =
        HashMap::new();

    // Normalize user pattern, user hw profile, user hw properties
    for pattern in pattern_element_vec {
        if let Ok(quantity) = pattern.parse::<u8>() {
            /* for i in 1..=quantity {
                dessired_hsm_node_hw_profile_vec.push(pattern_node_type_hashset.clone());
            } */
            pattern_node_type_vec.dedup(); // Normalize user hw properties by deduplicating
            user_defined_hw_properties_grouped_by_hw_profile_vec_sorted
                .push(pattern_node_type_vec.clone());
            user_defined_hw_profile_qty_hashmap.insert(pattern_node_type_vec.join(":"), quantity);
            pattern_node_type_vec = Vec::new();
        } else {
            if !pattern_node_type_vec.contains(&pattern.to_string()) {
                // Normalize user hw
                // properties by
                // ignoring duplicates
                // Avoid inserting duplicate
                // hw properties
                pattern_node_type_vec.push(pattern.to_string());
            }
        }
    }

    /* println!(
        "user_defined_hw_profile_qty_hashmap:\n{:#?}",
       user_defined_hw_profile_qty_hashmap
    ); */

    user_defined_hw_properties_grouped_by_hw_profile_vec_sorted
        .sort_by(|a, b| b.len().cmp(&a.len())); // sorting by number os strings in
                                                // each hw profile, eg we preffer [{a100, epyc}, {epyc} ] to [ {epyc}, {a100, epyc} ],
                                                // because most nodes will end up in the first hw property (that matches)
                                                // and we want to restrict the hw profile match with the node hw inventory
                                                // coming from CSM as much as we can ...

    // Target HSM group
    let target_hsm_group_value =
        hsm::http_client::get_hsm_group(shasta_token, shasta_base_url, target_hsm_group_name)
            .await
            .unwrap();

    let hsm_group_parent_members =
        hsm::utils::get_members_from_hsm_group_serde_value(&target_hsm_group_value);

    let start = Instant::now();

    let mut nodes_hw_properties_from_user_pattern_tuple_vec: Vec<(String, Option<Vec<String>>)> =
        Vec::new(); // List of tuples
                    // that relates
                    // the node with
                    // the list of
                    // hw properties
                    // defined by the
                    // user

    let mut target_hsm_hw_pattern_summary = HsmHwPatternSummary {
        user_defined_hw_profile_vec: Vec::new(),
        user_defined_hw_profile_vec_hw_prop_vec_sorted:
            user_defined_hw_properties_grouped_by_hw_profile_vec_sorted.clone(),
        node_counter_vec: Vec::new(),
    };

    let mut nodes_hw_properties_from_user_pattern_hashmap: HashMap<String, Vec<String>> =
        HashMap::new(); // HashMap with
                        // relationship
                        // between nodes
                        // and the list
                        // of hw properties defined by the user

    let mut tasks = tokio::task::JoinSet::new();

    let sem = Arc::new(Semaphore::new(5)); // CSM 1.3.1 higher number of concurrent tasks won't
                                           // make it faster

    for hsm_member in hsm_group_parent_members {
        let shasta_token_string = shasta_token.to_string();
        let shasta_base_url_string = shasta_base_url.to_string();
        let user_defined_hw_profile_vec_aux =
            user_defined_hw_properties_grouped_by_hw_profile_vec_sorted.clone();
        
        let permit = Arc::clone(&sem).acquire_owned().await;

        tasks.spawn(async move {
            let _permit = permit; // Wait semaphore to allow new tasks https://github.com/tokio-rs/tokio/discussions/2648#discussioncomment-34885
            hsm_node_hw_profile(
                shasta_token_string,
                shasta_base_url_string,
                &hsm_member,
                user_defined_hw_profile_vec_aux,
            )
            .await
        });
    }

    while let Some(message) = tasks.join_next().await {
        // println!("node_hw_pattern_tuple: {:?}", message);
        if let Ok(node_hw_property_tuple) = message {
            let node = node_hw_property_tuple.0.clone();
            let hw_property_vec = node_hw_property_tuple.1.clone().unwrap_or(Vec::new());
            let hw_profile_key_vec; // Used as hasmap key
            if hw_property_vec.is_empty() {
                // Node hw inventory did not match any property (property is a subset of a hw
                // profile, eg a100:epyc is a hw profile, then a100 is a property)
            } else {
                if hw_property_vec.len() > 1 {
                    // Node hw inventory matches more than 1 property, because we are in apply hsm
                    // based on node quantity, we treat all properties within a hw profile as being
                    // exclusive (eg property1 AND property2 AND ...) a node hw inventory needs to
                    // match all properties in a hw profile defined by the user.

                    hw_profile_key_vec = [
                        hw_property_vec
                            .clone()
                            .into_iter()
                            .filter(|hw_property| {
                                user_defined_hw_properties_grouped_by_hw_profile_vec_sorted
                                    .contains(&vec![hw_property.clone()].to_vec())
                            })
                            .collect(),
                        [hw_property_vec.join(":")].to_vec(),
                    ]
                    .concat();
                } else {
                    // node_hw_pattern_tuple.1.unwrap().len() == 1
                    // Node hw inventory matches only 1 property, so we want to also include nodes
                    // mathing a hw profile including this property

                    hw_profile_key_vec = vec![hw_property_vec.first().unwrap().to_string()];
                }

                for hw_profile_key in hw_profile_key_vec {
                    if user_defined_hw_profile_target_hsm_members_hashmap
                        .contains_key(&hw_profile_key)
                    {
                        user_defined_hw_profile_target_hsm_members_hashmap
                            .get_mut(&hw_profile_key)
                            .unwrap()
                            .push(node.clone());
                    } else {
                        user_defined_hw_profile_target_hsm_members_hashmap
                            .insert(hw_profile_key.clone(), vec![node.clone()]);
                    }
                }
            }
            // println!("Hw profile for {} is: {:?}", node, hw_profile_key);
            nodes_hw_properties_from_user_pattern_hashmap.insert(node, hw_property_vec);
            nodes_hw_properties_from_user_pattern_tuple_vec.push(node_hw_property_tuple.clone());
            target_hsm_hw_pattern_summary.insert_node_hw_profile_counter(
                node_hw_property_tuple.0,
                node_hw_property_tuple.1.clone().unwrap_or(Vec::new()),
                1,
            );
        } else {
            log::error!("Failed procesing/fetching node hw information");
        }
    }

    let duration = start.elapsed();
    println!(
        "Time elapsed to calculate actual_hsm_node_hw_profile_vec in '{}' is: {:?}",
        target_hsm_group_name, duration
    );

    /*     println!(
        "hsm_hw_pattern_summary: \n{:?}",
        target_hsm_hw_pattern_summary
    ); */

    /*     println!(
        "nodes_hw_properties_from_user_pattern_tuple_vec: \n{:?}",
        nodes_hw_properties_from_user_pattern_tuple_vec
    ); */

    /*     println!(
        "user_defined_hw_profile_target_hsm_members: {:#?}",
        user_defined_hw_profile_target_hsm_members_hashmap
    ); */

    /* println!("nodes_patterns_vec: {:#?}", nodes_patterns_vec);
    println!("nodes_patterns_hashmap: {:#?}", nodes_patterns_hashmap); */

    // Print table
    /*     let count_patterns = count_patterns(
        &user_defined_hw_properties_grouped_by_hw_profile_vec_sorted,
        &nodes_hw_properties_from_user_pattern_tuple_vec,
    );
    println!("count_pattern: {:?}", count_patterns.clone()); */

    utils::print_table(target_hsm_hw_pattern_summary.clone());

    // Calculate nodes in HSM group to keep
    /*     for (index, user_defined_hw_profile) in user_defined_hw_profile_vec_sorted.iter().enumerate() {
        let counter: u8 = count_patterns.iter().map(|patterns| patterns[index]).sum();
        println!(
            "# Pattern: {:?} counters: {}",
            user_defined_hw_profile, counter
        );
    } */

    // Free node HSM group
    let hsm_group_parent_value =
        hsm::http_client::get_hsm_group(shasta_token, shasta_base_url, hsm_group_parent)
            .await
            .unwrap();

    let hsm_group_parent_members =
        hsm::utils::get_members_from_hsm_group_serde_value(&hsm_group_parent_value);

    let start = Instant::now();

    let mut actual_hsm_node_hw_profile_vec: Vec<(String, Option<Vec<String>>)> = Vec::new();

    let mut tasks = tokio::task::JoinSet::new();

    let mut free_nodes_hsm_hw_pattern_summary = HsmHwPatternSummary {
        user_defined_hw_profile_vec: Vec::new(),
        user_defined_hw_profile_vec_hw_prop_vec_sorted:
            user_defined_hw_properties_grouped_by_hw_profile_vec_sorted.clone(),
        node_counter_vec: Vec::new(),
    };

    for hsm_member in hsm_group_parent_members.clone() {
        let shasta_token_string = shasta_token.to_string();
        let shasta_base_url_string = shasta_base_url.to_string();
        let user_defined_hw_profile_vec_aux =
            user_defined_hw_properties_grouped_by_hw_profile_vec_sorted.clone();
        tasks.spawn(async move {
            hsm_node_hw_profile(
                shasta_token_string,
                shasta_base_url_string,
                &hsm_member,
                user_defined_hw_profile_vec_aux,
            )
            .await
        });
    }

    while let Some(message) = tasks.join_next().await {
        if let Ok(node_hw_property_tuple) = message {
            let node = node_hw_property_tuple.0.clone();
            let hw_profile_key = node_hw_property_tuple.1.clone().unwrap().join(":");
            if user_defined_hw_profile_hsm_free_node_members_hashmap.contains_key(&hw_profile_key) {
                user_defined_hw_profile_hsm_free_node_members_hashmap
                    .get_mut(&hw_profile_key)
                    .unwrap()
                    .push(node.clone());
            } else {
                user_defined_hw_profile_hsm_free_node_members_hashmap
                    .insert(hw_profile_key.clone(), vec![node.clone()]);
            }
            // println!("Hw profile for {} is: {:?}", node, hw_profile_key);
            actual_hsm_node_hw_profile_vec.push(node_hw_property_tuple.clone());
            free_nodes_hsm_hw_pattern_summary.insert_node_hw_profile_counter(
                node_hw_property_tuple.0,
                node_hw_property_tuple.1.clone().unwrap_or(Vec::new()),
                1,
            );
        } else {
            log::error!("Failed procesing/fetching node hw information");
        }
    }

    let duration = start.elapsed();
    println!(
        "Time elapsed to calculate actual_hsm_node_hw_profile_vec in '{}' is: {:?}",
        hsm_group_parent, duration
    );

    /*     println!(
        "user_defined_hw_profile_hsm_free_node_members: {:#?}",
        user_defined_hw_profile_hsm_free_node_members_hashmap
    ); */

    /*     println!(
        "HSM parent {} - pattern relation: {:#?}",
        hsm_group_parent, actual_hsm_node_hw_profile_vec
    ); */

    utils::print_table(free_nodes_hsm_hw_pattern_summary.clone());

    // Summary
    let mut nodes_to_remove_from_target_hsm_group;
    let mut nodes_to_add_to_target_hsm_group;
    let mut target_hsm_group_members =
        hsm::utils::get_members_from_hsm_group_serde_value(&target_hsm_group_value);
    let mut parent_hsm_group_members = hsm_group_parent_members.clone();

    let hs_profile_total_counters = target_hsm_hw_pattern_summary.get_hw_profile_total_counters();
    for (user_defined_hw_profile, total_counter) in hs_profile_total_counters {
        let diff_nodes_hw_profile: i8 = (*user_defined_hw_profile_qty_hashmap
            .get(&user_defined_hw_profile)
            .unwrap() as i16
            - total_counter as i16)
            .try_into()
            .unwrap();

        if diff_nodes_hw_profile > 0 {
            println!(
                "Regarding hw profile {} --> add {} nodes",
                user_defined_hw_profile,
                diff_nodes_hw_profile.abs()
            );

            nodes_to_add_to_target_hsm_group = free_nodes_hsm_hw_pattern_summary
                .get_candidate_nodes_with_specific_hw_profile(
                    &user_defined_hw_profile,
                    diff_nodes_hw_profile.abs() as u8,
                );

            if (nodes_to_add_to_target_hsm_group.len() as i8) < diff_nodes_hw_profile.abs() {
                eprintln!(
                    "Not enough nodes ({}) in '{}', it needs {}. Cancelling transaction.",
                    nodes_to_add_to_target_hsm_group.len(),
                    hsm_group_parent,
                    diff_nodes_hw_profile.abs()
                );
                std::process::exit(-1);
            } else {
                println!("Nodes to add: {:?}", nodes_to_add_to_target_hsm_group);
            }

            // Add nodes from parent/free node pool HSM group to target HSM group
            target_hsm_group_members = [
                target_hsm_group_members,
                nodes_to_add_to_target_hsm_group.clone(),
            ]
            .concat();

            // Remove nodes from parent/free node pool HSM group
            parent_hsm_group_members
                .retain(|xname| !nodes_to_add_to_target_hsm_group.contains(xname));
        } else if diff_nodes_hw_profile < 0 {
            println!(
                "Regarding hw profile {} --> remove {} nodes",
                user_defined_hw_profile,
                diff_nodes_hw_profile.abs()
            );

            nodes_to_remove_from_target_hsm_group = target_hsm_hw_pattern_summary
                .get_candidate_nodes_with_specific_hw_profile(
                    &user_defined_hw_profile,
                    diff_nodes_hw_profile.abs() as u8,
                );

            println!(
                "Nodes to remove: {:?}",
                nodes_to_remove_from_target_hsm_group
            );

            // Remove nodes from target HSM group
            target_hsm_group_members
                .retain(|xname| !nodes_to_remove_from_target_hsm_group.contains(xname));

            // Add nodes rom target HSM group to parent/free pool target HSM group
            parent_hsm_group_members = [
                parent_hsm_group_members,
                nodes_to_remove_from_target_hsm_group,
            ]
            .concat();
        } else {
            println!(
                "Regarding hw profile {} --> no changes",
                user_defined_hw_profile
            );
        }
    }

    target_hsm_group_members.sort();
    parent_hsm_group_members.sort();
    println!(
        "{} HSM group:\n{:?}",
        target_hsm_group_name, target_hsm_group_members
    );
    println!(
        "{} HSM group:\n{:?}",
        hsm_group_parent, parent_hsm_group_members
    );
}

pub mod utils {

    use serde_json::Value;

    use crate::shasta::hsm;

    pub async fn hsm_node_hw_profile(
        shasta_token: String,
        shasta_base_url: String,
        hsm_member: &str,
        user_defined_hw_profile_vec: Vec<Vec<String>>,
    ) -> (String, Option<Vec<String>>) {
        let profile =
            hsm::http_client::get_hw_inventory(&shasta_token, &shasta_base_url, hsm_member)
                .await
                .unwrap();
        let actual_xname_hw_profile_hashset =
            get_node_hw_properties(&profile, user_defined_hw_profile_vec.clone());

        /* println!(
            "Node {} matches node pattern {:?}",
            hsm_member, actual_xname_hw_profile_hashset
        ); */

        (hsm_member.to_string(), actual_xname_hw_profile_hashset)
    }

    pub fn count_patterns(
        user_defined_hw_properties_grouped_by_hw_profile_vec_sorted: &Vec<Vec<String>>,
        nodes_hw_properties_from_user_pattern_vec: &Vec<(String, Option<Vec<String>>)>,
    ) -> Vec<(String, Vec<u8>)> {
        let mut counters = Vec::new();

        for node_hw_summary in nodes_hw_properties_from_user_pattern_vec {
            let node_property_vec = node_hw_summary.1.clone().unwrap_or(Vec::new());
            let mut hw_property_counter_vec: Vec<u8> = Vec::new();
            for user_property in user_defined_hw_properties_grouped_by_hw_profile_vec_sorted {
                hw_property_counter_vec.push(check_node_complains_pattern(
                    user_property,
                    &node_property_vec,
                ));
            }
            counters.push((node_hw_summary.0.clone(), hw_property_counter_vec));
        }

        counters
    }

    pub fn check_node_complains_pattern(
        user_property_vec: &Vec<String>,
        node_property_vec: &Vec<String>,
    ) -> u8 {
        if !node_property_vec.is_empty() {
            if user_property_vec
                .iter()
                .all(|user_property| node_property_vec.clone().contains(user_property))
            {
                1
            } else {
                0
            }
        } else {
            0
        }
    }

    pub fn print_table(hsm_hw_pattern_summary: super::HsmHwPatternSummary) {
        let user_patterns = hsm_hw_pattern_summary.user_defined_hw_profile_vec_hw_prop_vec_sorted;
        let mut nodes_pattern_summary_vec = hsm_hw_pattern_summary.node_counter_vec;

        nodes_pattern_summary_vec.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap());

        // Print table
        let mut headers: Vec<Vec<String>> = [Vec::new()].to_vec();
        headers[0].push("Node".to_string());
        headers = headers
            .into_iter()
            .chain(user_patterns.clone().into_iter())
            .collect();

        let mut table = comfy_table::Table::new();

        table.set_header(headers.iter().map(|header| header.join(":")));

        for node_pattern_summary in nodes_pattern_summary_vec {
            let xname = node_pattern_summary.0.to_string();
            let node_pattern_vec = node_pattern_summary.1.clone();
            /*             println!(
                "### Comparing node pattern {:?} and user patterns {:?}",
                node_pattern_option, user_patterns
            ); */
            let mut row: Vec<comfy_table::Cell> = Vec::new();
            row.push(
                comfy_table::Cell::new(xname.clone())
                    .set_alignment(comfy_table::CellAlignment::Center),
            );
            for node_pattern in node_pattern_vec {
                // println!("Compare user_pattern {:?} and node_pattern_option: {:?}", user_pattern, node_pattern_option);
                if node_pattern > 0 {
                    row.push(
                        comfy_table::Cell::new("✅".to_string())
                            .set_alignment(comfy_table::CellAlignment::Center),
                    );
                } else {
                    row.push(
                        comfy_table::Cell::new("❌".to_string())
                            .set_alignment(comfy_table::CellAlignment::Center),
                    );
                }
                /* if let Some(node_pattern) = &node_pattern_option {
                    if user_pattern
                        .iter()
                        .all(|pattern| node_pattern.clone().contains(pattern))
                    {
                    } else {
                        row.push(
                            comfy_table::Cell::new("❌".to_string())
                                .set_alignment(comfy_table::CellAlignment::Center),
                        );
                    }

                } else {
                    row.push(
                        comfy_table::Cell::new("❌".to_string())
                            .set_alignment(comfy_table::CellAlignment::Center),
                    );
                } */
            }
            table.add_row(row);
        }

        println!("{table}");
    }

    /// Returns the properties in hw_property_list found in the node_hw_inventory_value
    pub fn get_node_hw_properties(
        node_hw_inventory_value: &Value,
        mut hw_property_list: Vec<Vec<String>>,
    ) -> Option<Vec<String>> {
        // println!("patterns: {:?}", patterns_hw_inv_vec);
        hw_property_list.sort_by(|a, b| b.len().cmp(&a.len())); // sorting by number os strings in
                                                                   // each pattern, eg we preffer [
                                                                   // {a100, epyc},
                                                                   // {epyc} ] to [ {epyc}, {a100, epyc} ], because most nodes will end up in the first pattern and we want to restrict the pattern match with the node inventory coming from CSM as much as we can ...
        /* println!(
            "sorting patterns within same node by... patterns size??? {:#?}",
            hw_property_list
        ); */

        let processor_vec =
            get_list_processor_model_from_hw_inventory_value(&node_hw_inventory_value)
                .unwrap_or_default();

        let accelerator_vec =
            get_list_accelerator_model_from_hw_inventory_value(&node_hw_inventory_value)
                .unwrap_or_default();

        let processor_and_accelerator_concat = [ processor_vec.concat(), accelerator_vec.concat() ].concat().to_lowercase();

        for pattern_hw_inv_vec in hw_property_list {
            if pattern_hw_inv_vec.iter().all(|pattern| processor_and_accelerator_concat.contains(pattern)) {
                return Some(pattern_hw_inv_vec);
            }
        }

        None
    }

    pub fn get_list_processor_model_from_hw_inventory_value(
        hw_inventory: &Value,
    ) -> Option<Vec<String>> {
        hw_inventory["Nodes"].as_array().unwrap().first().unwrap()["Processors"]
            .as_array()
            .map(|processor_list: &Vec<Value>| {
                processor_list
                    .iter()
                    .map(|processor| {
                        processor
                            .pointer("/PopulatedFRU/ProcessorFRUInfo/Model")
                            .unwrap()
                            .to_string()
                    })
                    .collect::<Vec<String>>()
            })
    }

    pub fn get_list_accelerator_model_from_hw_inventory_value(
        hw_inventory: &Value,
    ) -> Option<Vec<String>> {
        hw_inventory["Nodes"].as_array().unwrap().first().unwrap()["NodeAccels"]
            .as_array()
            .map(|accelerator_list| {
                accelerator_list
                    .iter()
                    .map(|accelerator| {
                        accelerator
                            .pointer("/PopulatedFRU/NodeAccelFRUInfo/Model")
                            .unwrap()
                            .to_string()
                    })
                    .collect::<Vec<String>>()
            })
    }
}

use clap::{arg, value_parser, ArgAction, ArgGroup, Command};

use std::path::PathBuf;

pub fn subcommand_get_node(hsm_group: Option<&String>) -> Command {
    let mut get_node = Command::new("nodes")
        .aliases(["n", "node", "nd"])
        .about("Get members of a HSM group")
        .arg(arg!(-n --"nids-only-one-line" "Prints nids in one line eg nidxxxxxx,nidyyyyyy,nidzzzzzz,..."))
        .arg(arg!(-x --"xnames-only-one-line" "Prints xnames in one line eg x1001c1s5b0n0,x1001c1s5b0n1,..."))
        .arg(arg!(-o --output <FORMAT> "Output format. If missing it will print output data in human redeable (tabular) format").value_parser(["json"]));

    match hsm_group {
        None => {
            get_node = get_node
                .arg_required_else_help(true)
                .arg(arg!(<HSM_GROUP_NAME> "hsm group name"))
        }
        Some(_) => {}
    }

    get_node
}

pub fn subcommand_get_hsm_groups_details(hsm_group: Option<&String>) -> Command {
    let mut get_hsm_group = Command::new("hsm-groups")
        .aliases(["h", "hg", "hsm", "hsmgrps"])
        .about("Get HSM groups details");

    match hsm_group {
        None => {
            get_hsm_group = get_hsm_group
                .arg_required_else_help(true)
                .arg(arg!(<HSM_GROUP_NAME> "hsm group name"))
        }
        Some(_) => {
            get_hsm_group = get_hsm_group.arg_required_else_help(false);
        }
    }

    get_hsm_group
}

pub fn subcommand_apply_cluster(/* hsm_group: Option<&String> */) -> Command {
    Command::new("cluster")
        .aliases(["clus","clstr"])
        .arg_required_else_help(true)
        .about("Create a CFS configuration, a CFS image, a BOS sessiontemplate and a BOS session")
        .arg(arg!(-f --file <SAT_FILE> "SAT file with CFS configuration, CFS image and BOS session template details").value_parser(value_parser!(PathBuf)))
        .arg(arg!(-t --tag <VALUE> "Tag added as a suffix in the CFS configuration name and CFS session name. If missing, then a default value will be used with timestamp"))
        .arg(arg!(-v --"ansible-verbosity" <VALUE> "Ansible verbosity. The verbose mode to use in the call to the ansible-playbook command.\n1 = -v, 2 = -vv, etc. Valid values range from 0 to 4. See the ansible-playbook help for more information.")
            .value_parser(["1", "2", "3", "4"])
            .num_args(1)
            // .require_equals(true)
            .default_value("2")
            .default_missing_value("2"))
        .arg(arg!(-p --"ansible-passthrough" <VALUE> "Additional parameters that are added to all Ansible calls for the session. This field is currently limited to the following Ansible parameters: \"--extra-vars\", \"--forks\", \"--skip-tags\", \"--start-at-task\", and \"--tags\". WARNING: Parameters passed to Ansible in this way should be used with caution. State will not be recorded for components when using these flags to avoid incorrect reporting of partial playbook runs."))
        .arg(arg!(-o --output <FORMAT> "Output format. If missing it will print output data in human redeable (tabular) format").value_parser(["json"]))
}

pub fn subcommand_apply_node_on(hsm_group: Option<&String>) -> Command {
    let mut apply_node_on = Command::new("on")
        .about("Start nodes")
        .arg_required_else_help(true)
        .arg(arg!(<XNAMES> "nodes' xnames"))
        .arg(arg!(-r --reason <TEXT> "reason to power on"));

    match hsm_group {
        None => {
            apply_node_on = apply_node_on
                .arg(arg!(-H --"hsm-group" <HSM_GROUP_NAME> "hsm group name"))
                .group(
                    ArgGroup::new("hsm-group_or_xnames")
                        .args(["hsm-group", "XNAMES"])
                        .multiple(true),
                )
        }
        Some(_) => {}
    };

    apply_node_on
}

pub fn subcommand_apply_node_off(hsm_group: Option<&String>) -> Command {
    let mut apply_node_off = Command::new("off")
        .arg_required_else_help(true)
        .about("Shutdown nodes")
        .arg(arg!(<XNAMES> "nodes' xnames"))
        .arg(arg!(-f --force "force").action(ArgAction::SetTrue))
        .arg(arg!(-r --reason <TEXT> "reason to power off"));

    match hsm_group {
        None => {
            apply_node_off = apply_node_off
                .arg(arg!(-H --"hsm-group" <HSM_GROUP_NAME> "hsm group name"))
                .group(
                    ArgGroup::new("hsm-group_or_xnames")
                        .args(["hsm-group", "XNAMES"])
                        .multiple(true),
                )
        }
        Some(_) => {}
    };

    apply_node_off
}

pub fn subcommand_apply_node_reset(hsm_group: Option<&String>) -> Command {
    let mut apply_node_reset = Command::new("reset")
        .aliases(["r", "res", "rst", "restart", "rstrt"])
        .arg_required_else_help(true)
        .about("Restart nodes")
        .arg(arg!(<XNAMES> "nodes' xnames"))
        .arg(arg!(-f --force "force").action(ArgAction::SetTrue))
        .arg(arg!(-r --reason <TEXT> "reason to reset"));

    match hsm_group {
        None => {
            apply_node_reset = apply_node_reset
                .arg(arg!(-H --"hsm-group" <HSM_GROUP_NAME> "hsm group name"))
                .group(
                    ArgGroup::new("hsm-group_or_xnames")
                        .args(["hsm-group", "XNAMES"])
                        .multiple(true),
                )
        }
        Some(_) => {}
    };

    apply_node_reset
}

pub fn subcommand_apply_hsm() -> Command {
    Command::new("hsm-group")
        .aliases(["hsm"])
        .arg_required_else_help(true)
        .about("WIP - UNSTABLE!!! Rearange nodes in a HSM group based on pattern")
        .arg(arg!(-p --pattern <VALUE> ... "Pattern to express the new HSM layout like `<hsm_group_name>[:<property>]*:<num_nodes>`. Where hsm_group_name (mandatory) is the target HSM group, property (optional) is the property (eg NVIDIA, A100, AMD, EPYC, etc) to filter nodes' components (Nodes[].Processors[].PopulatedFRU.ProcessorFRUInfo.Model or Nodes[].NodeAccels[].PopulatedFRU.NodeAccelFRUInfo.Model) and num_nodes (mandatory) is the number of nodes with those properties we need for the new HSM layout. Eg test:nvidia:a100:2 means `test` HSM group should have 2 nodes with NVIDIA A100, test:nvidia:2:amd:rome:3 means `test` HSM group will have 2 nvidia nodes and 3 AMD ROME nodes. NOTE: a single pattern may match multiple nodes therefore the total combination of num_nodes for a single HSM group does not accumulate.").required(true))
}

pub fn subcommand_update_nodes(hsm_group: Option<&String>) -> Command {
    let mut update_nodes = Command::new("nodes")
        .aliases(["n", "node", "nd"])
        .arg_required_else_help(true)
        .about("Updates boot and configuration of a group of nodes. Boot configuration means updating the image used to boot the machine. Configuration of a node means the CFS configuration with the ansible scripts running once a node has been rebooted.\neg:\nmanta update hsm-group --boot-image <boot cfs configuration name> --desired-configuration <desired cfs configuration name>")
        .arg(arg!(-b --"boot-image" <CFS_CONFIG> "CFS configuration name related to the image to boot the nodes"))
        .arg(arg!(-d --"desired-configuration" <CFS_CONFIG> "CFS configuration name to configure the nodes after booting"));

    update_nodes = update_nodes
        .arg(arg!(<XNAMES> "Comma separated list of xnames which boot image will be updated"));

    /* update_nodes = update_nodes
    .arg(arg!(<CFS_CONFIG> "CFS configuration name used to boot and configure the nodes")); */

    update_nodes = match hsm_group {
        Some(_) => update_nodes,
        None => update_nodes.arg(arg!([HSM_GROUP_NAME] "hsm group name, this field should be used to validate the XNAMES belongs to HSM_GROUP_NAME")),
    };

    /* update_nodes = update_nodes.group(
        ArgGroup::new("boot-image_or_desired-configuration")
            .args(["boot-image", "desired-configuration"]),
    ); */

    /* update_nodes = update_nodes.groups([
        ArgGroup::new("update-node-boot_or_update-node-desired-configuration")
            .args(["boot", "desired-configuration"]),
        ArgGroup::new("update-node-args").args(["XNAMES", "CFS_CONFIG"]),
    ]); */

    /* update_node = update_node
        .group(
            ArgGroup::new("boot_and_config")
                .args(["boot-image", "configuration"])
                .required(true),
        )
        .group(ArgGroup::new("config").args(["CFS_CONFIG"]));

    update_node = update_node.group(ArgGroup::new("boot-config_or_config").args(["boot_and_config", "config"])); */

    update_nodes
}

pub fn subcommand_update_hsm_group(hsm_group: Option<&String>) -> Command {
    let mut update_hsm_group = Command::new("hsm-group")
        .aliases(["h", "hsm"])
        .arg_required_else_help(true)
        .about("Updates boot and configuration of all the nodes in a HSM group. Boot configuration means updating the image used to boot the machine. Configuration of a node means the CFS configuration with the ansible scripts running once a node has been rebooted.\neg:\nmanta update hsm-group --boot-image <boot cfs configuration name> --desired-configuration <desired cfs configuration name>")
        .arg(arg!(-b --"boot-image" <CFS_CONFIG> "CFS configuration name related to the image to boot the nodes"))
        .arg(arg!(-d --"desired-configuration" <CFS_CONFIG> "CFS configuration name to configure the nodes after booting"));

    update_hsm_group = match hsm_group {
        Some(_) => update_hsm_group,
        None => update_hsm_group.arg(arg!(<HSM_GROUP_NAME> "HSM group name").required(true)),
    };

    update_hsm_group
}

pub fn build_cli(hsm_group: Option<&String>) -> Command {
    Command::new("manta")
        .arg_required_else_help(true)
        .subcommand(
            Command::new("get")
                .alias("g")
                .arg_required_else_help(true)
                .about("Get cluster details")
                .subcommand(subcommand_get_node(hsm_group))
                .subcommand(subcommand_get_hsm_groups_details(hsm_group))
        )
        .subcommand(
            Command::new("apply")
                .alias("a")
                .arg_required_else_help(true)
                .about("Create new cluster")
                .subcommand(subcommand_apply_cluster(/* hsm_group */))
                .subcommand(subcommand_apply_hsm(/* hsm_group */))
                .subcommand(
                    Command::new("node")
                        .aliases(["n", "nod"])
                        .arg_required_else_help(true)
                        .about("Make changes to nodes")
                        .subcommand(subcommand_apply_node_on(hsm_group))
                        .subcommand(subcommand_apply_node_off(hsm_group))
                        .subcommand(subcommand_apply_node_reset(hsm_group)),
                ),
        )
        .subcommand(
            Command::new("update")
                .alias("u")
                .arg_required_else_help(true)
                .about("Update existing cluster details")
                .subcommand(subcommand_update_nodes(hsm_group))
                .subcommand(subcommand_update_hsm_group(hsm_group)),
        )
}

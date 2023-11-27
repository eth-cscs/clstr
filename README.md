# CLSTR

cli based to manage HSM groups.

## Run

Get hardware inventory for hsm group zinal

```
cargo run -- get hsm artifacts zinal
```

Get xnames for a hsm group called zinal which should have minimum:
 - x4 A100 Nvidia gpus
 - x30 epyc AMD cpus
 - x2 instinct AMD gpus

```
cargo run -- a hsm -p zinal:a100:4:epyc:30:instinct:2
```

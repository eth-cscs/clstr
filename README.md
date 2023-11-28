# ⚠️  ⚠️  ⚠️  WIP all this is subject to change ⚠️  ⚠️  ⚠️ 

# CLSTR

Cli based to manage HSM groups. The integration with CSM is provided by [mesa](https://github.com/eth-cscs/mesa)

# Introduction

Clstr works around the concept of resurce pools which is a group of hardware components that can be allocated to a tenant, workload, etc. A resource pool can be also called a cluster or a HSM group.

## Examples

### Get hardware inventory for hsm group zinal

```
$ clstr get hsm artifacts zinal
+---------------+-----------+-----------+---------------------------------+---------------------------------+---------------------------------+---------------------------------+--------+-----------------------+------------------------------------+
| Node          | 16384 MiB | 65536 MiB | AMD EPYC 7713 64-Core Processor | AMD EPYC 7742 64-Core Processor | AMD EPYC 7A53 64-Core Processor | AMD INSTINCT MI200 (MCM) OAM LC | ERROR  | NVIDIA_A100-SXM4-80GB | SS11 200Gb 2P NIC Mezz REV02 (HSN) |
+=====================================================================================================================================================================================================================================================+
| x1001c1s5b0n0 |  ✅ (16)  |     ❌    |                ❌               |              ✅ (2)             |                ❌               |                ❌               |   ❌   |           ❌          |               ✅ (1)               |
|---------------+-----------+-----------+---------------------------------+---------------------------------+---------------------------------+---------------------------------+--------+-----------------------+------------------------------------|
| x1001c1s5b0n1 |  ✅ (16)  |     ❌    |                ❌               |              ✅ (2)             |                ❌               |                ❌               |   ❌   |           ❌          |               ✅ (1)               |
|---------------+-----------+-----------+---------------------------------+---------------------------------+---------------------------------+---------------------------------+--------+-----------------------+------------------------------------|
| x1001c1s5b1n0 |  ✅ (16)  |     ❌    |                ❌               |              ✅ (2)             |                ❌               |                ❌               |   ❌   |           ❌          |               ✅ (1)               |
|---------------+-----------+-----------+---------------------------------+---------------------------------+---------------------------------+---------------------------------+--------+-----------------------+------------------------------------|
| x1001c1s5b1n1 |  ✅ (16)  |     ❌    |                ❌               |              ✅ (2)             |                ❌               |                ❌               |   ❌   |           ❌          |               ✅ (1)               |
|---------------+-----------+-----------+---------------------------------+---------------------------------+---------------------------------+---------------------------------+--------+-----------------------+------------------------------------|
| x1001c1s6b0n0 |  ✅ (15)  |     ❌    |                ❌               |              ✅ (2)             |                ❌               |                ❌               | ⚠️  (1) |           ❌          |               ✅ (1)               |
|---------------+-----------+-----------+---------------------------------+---------------------------------+---------------------------------+---------------------------------+--------+-----------------------+------------------------------------|
| x1001c1s6b0n1 |  ✅ (16)  |     ❌    |                ❌               |              ✅ (2)             |                ❌               |                ❌               |   ❌   |           ❌          |               ✅ (1)               |
|---------------+-----------+-----------+---------------------------------+---------------------------------+---------------------------------+---------------------------------+--------+-----------------------+------------------------------------|
| x1001c1s6b1n0 |  ✅ (16)  |     ❌    |                ❌               |              ✅ (2)             |                ❌               |                ❌               |   ❌   |           ❌          |               ✅ (1)               |
|---------------+-----------+-----------+---------------------------------+---------------------------------+---------------------------------+---------------------------------+--------+-----------------------+------------------------------------|
| x1001c1s6b1n1 |  ✅ (16)  |     ❌    |                ❌               |              ✅ (2)             |                ❌               |                ❌               |   ❌   |           ❌          |               ✅ (1)               |
|---------------+-----------+-----------+---------------------------------+---------------------------------+---------------------------------+---------------------------------+--------+-----------------------+------------------------------------|
| x1001c1s7b0n0 |  ✅ (16)  |     ❌    |                ❌               |              ✅ (2)             |                ❌               |                ❌               |   ❌   |           ❌          |               ✅ (1)               |
|---------------+-----------+-----------+---------------------------------+---------------------------------+---------------------------------+---------------------------------+--------+-----------------------+------------------------------------|
| x1001c1s7b0n1 |  ✅ (16)  |     ❌    |                ❌               |              ✅ (2)             |                ❌               |                ❌               |   ❌   |           ❌          |               ✅ (1)               |
|---------------+-----------+-----------+---------------------------------+---------------------------------+---------------------------------+---------------------------------+--------+-----------------------+------------------------------------|
| x1001c1s7b1n0 |  ✅ (16)  |     ❌    |                ❌               |              ✅ (2)             |                ❌               |                ❌               |   ❌   |           ❌          |               ✅ (1)               |
|---------------+-----------+-----------+---------------------------------+---------------------------------+---------------------------------+---------------------------------+--------+-----------------------+------------------------------------|
| x1001c1s7b1n1 |  ✅ (16)  |     ❌    |                ❌               |              ✅ (2)             |                ❌               |                ❌               |   ❌   |           ❌          |               ✅ (1)               |
|---------------+-----------+-----------+---------------------------------+---------------------------------+---------------------------------+---------------------------------+--------+-----------------------+------------------------------------|
| x1005c0s4b0n0 |     ❌    |   ✅ (8)  |              ✅ (1)             |                ❌               |                ❌               |                ❌               |   ❌   |         ✅ (4)        |               ✅ (4)               |
|---------------+-----------+-----------+---------------------------------+---------------------------------+---------------------------------+---------------------------------+--------+-----------------------+------------------------------------|
| x1005c0s4b0n1 |     ❌    |   ✅ (8)  |              ✅ (1)             |                ❌               |                ❌               |                ❌               |   ❌   |         ✅ (4)        |               ✅ (4)               |
|---------------+-----------+-----------+---------------------------------+---------------------------------+---------------------------------+---------------------------------+--------+-----------------------+------------------------------------|
| x1006c1s4b0n0 |     ❌    |   ✅ (8)  |                ❌               |                ❌               |              ✅ (1)             |              ✅ (8)             |   ❌   |           ❌          |               ✅ (4)               |
|---------------+-----------+-----------+---------------------------------+---------------------------------+---------------------------------+---------------------------------+--------+-----------------------+------------------------------------|
| x1006c1s4b1n0 |     ❌    |   ✅ (8)  |                ❌               |                ❌               |              ✅ (1)             |              ✅ (8)             |   ❌   |           ❌          |               ✅ (4)               |
+---------------+-----------+-----------+---------------------------------+---------------------------------+---------------------------------+---------------------------------+--------+-----------------------+------------------------------------+
```

### Get hardware resources of a node

```
$ clstr get nodes artifacts`` x1001c1s6b0n0 zinal
+---------------+------------------+----------------+------------------------------------+
| Node XName    | Component XName  | Component Type | Component Info                     |
+========================================================================================+
| x1001c1s6b0n0 | x1001c1s6b0n0p0  | Processor      | AMD EPYC 7742 64-Core Processor    |
|---------------+------------------+----------------+------------------------------------|
| x1001c1s6b0n0 | x1001c1s6b0n0p1  | Processor      | AMD EPYC 7742 64-Core Processor    |
|---------------+------------------+----------------+------------------------------------|
| x1001c1s6b0n0 | x1001c1s6b0n0d11 | Memory         | 16384 MiB                          |
|---------------+------------------+----------------+------------------------------------|
| x1001c1s6b0n0 | x1001c1s6b0n0d5  | Memory         | *** Missing info                   |
|---------------+------------------+----------------+------------------------------------|
| x1001c1s6b0n0 | x1001c1s6b0n0d7  | Memory         | 16384 MiB                          |
|---------------+------------------+----------------+------------------------------------|
| x1001c1s6b0n0 | x1001c1s6b0n0d2  | Memory         | 16384 MiB                          |
|---------------+------------------+----------------+------------------------------------|
| x1001c1s6b0n0 | x1001c1s6b0n0d0  | Memory         | 16384 MiB                          |
|---------------+------------------+----------------+------------------------------------|
| x1001c1s6b0n0 | x1001c1s6b0n0d12 | Memory         | 16384 MiB                          |
|---------------+------------------+----------------+------------------------------------|
| x1001c1s6b0n0 | x1001c1s6b0n0d8  | Memory         | 16384 MiB                          |
|---------------+------------------+----------------+------------------------------------|
| x1001c1s6b0n0 | x1001c1s6b0n0d14 | Memory         | 16384 MiB                          |
|---------------+------------------+----------------+------------------------------------|
| x1001c1s6b0n0 | x1001c1s6b0n0d15 | Memory         | 16384 MiB                          |
|---------------+------------------+----------------+------------------------------------|
| x1001c1s6b0n0 | x1001c1s6b0n0d1  | Memory         | 16384 MiB                          |
|---------------+------------------+----------------+------------------------------------|
| x1001c1s6b0n0 | x1001c1s6b0n0d3  | Memory         | 16384 MiB                          |
|---------------+------------------+----------------+------------------------------------|
| x1001c1s6b0n0 | x1001c1s6b0n0d13 | Memory         | 16384 MiB                          |
|---------------+------------------+----------------+------------------------------------|
| x1001c1s6b0n0 | x1001c1s6b0n0d9  | Memory         | 16384 MiB                          |
|---------------+------------------+----------------+------------------------------------|
| x1001c1s6b0n0 | x1001c1s6b0n0d6  | Memory         | 16384 MiB                          |
|---------------+------------------+----------------+------------------------------------|
| x1001c1s6b0n0 | x1001c1s6b0n0d10 | Memory         | 16384 MiB                          |
|---------------+------------------+----------------+------------------------------------|
| x1001c1s6b0n0 | x1001c1s6b0n0d4  | Memory         | 16384 MiB                          |
|---------------+------------------+----------------+------------------------------------|
| x1001c1s6b0n0 | x1001c1s6b0n0h0  | NodeHsnNic     | SS11 200Gb 2P NIC Mezz REV02 (HSN) |
+---------------+------------------+----------------+------------------------------------+
```

### Upscale or downscale a pool or resources

We need 2 pool of resources (target and parent) for clstr to work, we correlated a pool of resources with a CSM group. Clstr then will move all resources in target to parent, then allocate as much resources as the user expect back to the target hsm group.

Based on the example above, we now want to downscale the HSM group zinal to:

 - x4 A100 Nvidia gpus
 - x30 epyc AMD cpus
 - x2 instinct AMD gpus
 - 16GBx80 of RAM

Clstr cluster pattern tries to simplify the way to describe hw components in a cluster, the format is as follows: 

> `cluster name`:`hw component`:`quantity`(:`hw component`:`quantity`)*

The `<hw component>` is a string which clst is going to look for across all the hw components in either the target or parent HSM group, if works as a very simplified fuzzy finder.

Note: cluster pattern does not reflect compute nodes but the overall number of hw components you want in your cluster, this is important because a node with `NVIDIA_A100-SXM4-80GB` has 4 of them thefore if the user specifies `a100:2`, he/she will get 4 because it is the minimum a node can provide.

```
$ clstr apply hsm -p zinal:nvidia:2:mi200:4:7742:6:memory:80 

...

+---------------+--------+---------------------------------+---------------------------------+---------+--------+--------+
| Node          | 7742   | amd epyc 7713 64-core processor | amd epyc 7a53 64-core processor | memory  | mi200  | nvidia |
+========================================================================================================================+
| x1000c1s3b1n1 | ✅ (2) |                ❌               |                ❌               | ✅ (16) |   ❌   |   ❌   |
|---------------+--------+---------------------------------+---------------------------------+---------+--------+--------|
| x1000c1s7b1n0 | ✅ (2) |                ❌               |                ❌               | ✅ (16) |   ❌   |   ❌   |
|---------------+--------+---------------------------------+---------------------------------+---------+--------+--------|
| x1001c1s2b0n0 | ✅ (2) |                ❌               |                ❌               | ✅ (16) |   ❌   |   ❌   |
|---------------+--------+---------------------------------+---------------------------------+---------+--------+--------|
| x1005c0s4b0n1 |   ❌   |                ❌               |                ❌               | ✅ (32) |   ❌   | ✅ (4) |
|---------------+--------+---------------------------------+---------------------------------+---------+--------+--------|
| x1006c1s4b1n0 |   ❌   |                ❌               |                ❌               | ✅ (32) | ✅ (8) |   ❌   |
+---------------+--------+---------------------------------+---------------------------------+---------+--------+--------+

DEBUG - SOL - Target HSM 'zinal' members: x1000c1s3b1n1, x1001c1s5b0n0, x1001c1s5b0n1, x1005c1s0b0n1, x1006c0s1b1n0
```

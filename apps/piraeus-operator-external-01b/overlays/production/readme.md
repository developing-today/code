- type: nvme_optane / nvme_nand / sata_ssd / sata_mixed / sata_hdd
  - general type of storage drive
- latency: low_latency / medium_latency / high_latency
  - latency compared to other drives of the same type
- throughput: ultra_low_throughput / low_throughput / medium_throughput / high_throughput / ultra_high_throughput / max_throughput
  - throughput compared to other drives of the same type
  - by-type: consumer_or_lower, prosumer/medium_enterprise, above_medium_enterprise, max_performance_for_form_factor
- capacity: ultra_low_capacity / low_capacity / medium_capacity / high_capacity / ultra_high_capacity / max_capacity
  - max capacity per persistent volume
  - nvme: less_than_2, 2_to_4, more_than_4
  - hdd: less_than_12, 12_to_24, more_than_24

- 01b.us
  - boot drives
    - zwkrcejc
      - m.2 nvme 1TB Inland_TN320_NVMe_SSD_IB23AK0001P00505
    - hlzonshx
      - sata ssd 1TB Samsung_SSD_860_EVO_1TB_S3Z8NWAJC05002E
    - fpexlxgk
      - m.2 nvme 1TB (Inland) nvme-PCIe_SSD_23051610002029
  - linstor/drbd/extra drives
    - host*drives type
      - 2*1 optane-1.5-nvme (u.2 1.5TB - Intel Optane DC P4800X Series 1.5TB PCIe NVMe 2.5in U.2 SSD SSDPE21K015TA)
        - nvme_optane-medium_latency-medium_throughput-low_capacity
          - TODO
      - 3*1 medium-2.0-nvme (m.2 2.0TB - samsung 980 pro)
        - nvme_nand-medium_latency-low_throughput-medium_capacity
          - CREATED: storage-pool: xcp-sr-linstor_group_thin_device
      - 2*1 medium-1.0-nvme   (m.2 1.0TB - samsung 990 pro)
        - nvme_nand-medium_latency-medium_throughput-low_capacity
          - TODO
      - 2*2 medium-2.0-nvme   (m.2 2.0TB - hp fx900 pro)
        - nvme_nand-medium_latency-medium_throughput-medium_capacity
          - TODO

```
kubectl get pv
kubectl patch pv TODO_PV_NAME_HERE -p '{"spec":{"persistentVolumeReclaimPolicy":"Retain"}}'
```
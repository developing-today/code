- type: nvme_optane / nvme_nand / sata_ssd / sata_mixed / sata_hdd
  - general type of storage drive
- tier: basic / standard / premium / ultra
  - general tier of storage drive
- capacity: ultra_low_capacity / low_capacity / medium_capacity / high_capacity / ultra_high_capacity / max_capacity
  - max capacity per persistent volume low/medium/high
  - nvme: less_than_2, 2_to_4, more_than_4
  - hdd: less_than_12, 12_to_24, more_than_24
- linstor
  - basic.nvme.linstor-external-01b/
  - burst.nvme.linstor-external-01b/
  - sustain.nvme.linstor-external-01b/
  - optane.nvme.linstor-external-01b/
- immediate.delete.performance.nvme-ssd-mix.linstor-external-01b
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
        - optane.nvme.linstor-external-01b
          - TODO
      - 3*1 medium-2.0-nvme (m.2 2.0TB - samsung 980 pro)
        - basic.nvme.linstor-external-01b
          - CREATED: storage-pool: xcp-sr-linstor_group_thin_device
      - 2*1 medium-1.0-nvme   (m.2 1.0TB - samsung 990 pro)
        - burst.nvme.linstor-external-01b
          - TODO
      - 2*2 medium-4.0-nvme   (m.2 4.0TB - hp fx900 pro)
        - sustain.nvme.linstor-external-01b
          - TODO

```
kubectl get pv
kubectl patch pv TODO_PV_NAME_HERE -p '{"spec":{"persistentVolumeReclaimPolicy":"Retain"}}'
```

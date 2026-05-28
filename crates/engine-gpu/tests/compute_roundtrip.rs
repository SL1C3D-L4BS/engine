//! Phase 5.5 A.2b-i compute round-trip smoke test.
//!
//! Proves end-to-end GPU compute on real hardware: write data via the
//! Queue, dispatch a compute shader that consumes it through a
//! [`engine_gpu::BindGroup`], read the result back via
//! [`engine_gpu::Buffer::read_back`]. Validates the entire A.2b-i
//! surface (BindGroup construction, ComputePass::set_bind_group,
//! dispatch_workgroups, buffer copy + map) against an actual adapter.
//!
//! Skips gracefully on hosts without a Vulkan loader (same panic guard
//! ADR-074's `pipeline_smoke.rs` uses).

use std::panic::{self, AssertUnwindSafe};

use engine_gpu::{
    BindGroup, BindGroupDesc, BindGroupEntry, BindingResource, Buffer, BufferDesc, BufferUsage,
    CommandEncoder, ComputePipeline, ComputePipelineDesc, Device, DeviceLimits, ShaderModule,
    ShaderModuleDesc,
};

/// Trivial kernel: each thread writes `gid.x * 2` to the matching slot.
const KERNEL: &str = r#"
@group(0) @binding(0) var<storage, read_write> out : array<u32>;

@compute @workgroup_size(64, 1, 1)
fn cs_main(@builtin(global_invocation_id) gid : vec3<u32>) {
    if (gid.x < arrayLength(&out)) {
        out[gid.x] = gid.x * 2u;
    }
}
"#;

fn try_device() -> Option<Device> {
    let result = panic::catch_unwind(AssertUnwindSafe(|| {
        Device::new(DeviceLimits::Tier1Minimum, false)
    }));
    match result {
        Ok(Ok(d)) => Some(d),
        Ok(Err(e)) => {
            eprintln!("[compute_roundtrip] no compatible adapter (skipping): {e}");
            None
        }
        Err(_) => {
            eprintln!("[compute_roundtrip] wgpu has no backend enabled (skipping)");
            None
        }
    }
}

#[test]
fn bindgroup_dispatch_readback_round_trip() {
    let Some(device) = try_device() else {
        return;
    };
    let queue = device.queue();

    const COUNT: u32 = 64;
    let size = (COUNT as u64) * 4;

    // Storage buffer the kernel writes into. STORAGE for the shader binding;
    // COPY_SRC so we can copy into the readback staging buffer.
    let out_buffer = Buffer::new(
        &device,
        &BufferDesc {
            label: "compute_roundtrip.out",
            size,
            usage: BufferUsage::STORAGE | BufferUsage::COPY_SRC,
        },
    );

    // Readback staging buffer (MAP_READ + COPY_DST; STORAGE is incompatible
    // with MAP_READ in wgpu).
    let readback = Buffer::new(
        &device,
        &BufferDesc {
            label: "compute_roundtrip.readback",
            size,
            usage: BufferUsage::COPY_DST | BufferUsage::MAP_READ,
        },
    );

    // Pipeline + auto-derived bind-group layout (ADR-075 §8 A.2a path).
    let module = ShaderModule::new(
        &device,
        &ShaderModuleDesc {
            label: "compute_roundtrip.kernel",
            wgsl: KERNEL,
        },
    );
    let pipeline = ComputePipeline::new(
        &device,
        &ComputePipelineDesc {
            label: "compute_roundtrip.pipeline",
            layout: None,
            module: &module,
            entry_point: "cs_main",
        },
    );
    let layout = pipeline.bind_group_layout(0);

    // Bind group binding the storage buffer to @binding(0).
    let bind_group = BindGroup::new(
        &device,
        &BindGroupDesc {
            label: "compute_roundtrip.bindgroup",
            layout: &layout,
            entries: &[BindGroupEntry {
                binding: 0,
                resource: BindingResource::Buffer(&out_buffer),
            }],
        },
    );

    // Encode the dispatch + the copy into the readback buffer.
    let mut encoder = CommandEncoder::new(&device, "compute_roundtrip.encoder");
    {
        let mut cpass = encoder.begin_compute_pass("compute_roundtrip.pass");
        cpass.set_pipeline(&pipeline);
        cpass.set_bind_group(0, &bind_group);
        // 64 threads in one workgroup match the COUNT.
        cpass.dispatch_workgroups(1, 1, 1);
    }
    encoder.copy_buffer_to_buffer(&out_buffer, 0, &readback, 0, size);
    let _token = queue.submit(encoder);

    // Read back + decode.
    let bytes = readback.read_back().expect("readback succeeds");
    assert_eq!(bytes.len() as u64, size);
    let result: Vec<u32> = bytes
        .chunks_exact(4)
        .map(|c| u32::from_le_bytes([c[0], c[1], c[2], c[3]]))
        .collect();

    // Each slot must equal index * 2.
    for (i, v) in result.iter().enumerate() {
        assert_eq!(
            *v,
            (i as u32) * 2,
            "slot {i}: shader output {v}, expected {expected}",
            expected = (i as u32) * 2
        );
    }
}

#[test]
fn write_then_read_round_trips_via_compute() {
    // Variant: pre-fill via queue.write_buffer, kernel adds 1000 to each
    // slot, read back, assert sum. Exercises Queue::write_buffer on the
    // STORAGE path (which differs from COPY_DST-only readback buffers).
    let Some(device) = try_device() else {
        return;
    };
    let queue = device.queue();

    const COUNT: u32 = 64;
    let size = (COUNT as u64) * 4;

    let storage = Buffer::new(
        &device,
        &BufferDesc {
            label: "addk.storage",
            size,
            usage: BufferUsage::STORAGE | BufferUsage::COPY_SRC | BufferUsage::COPY_DST,
        },
    );
    let readback = Buffer::new(
        &device,
        &BufferDesc {
            label: "addk.readback",
            size,
            usage: BufferUsage::COPY_DST | BufferUsage::MAP_READ,
        },
    );

    // Seed the storage buffer with 0..64.
    let initial: Vec<u8> = (0u32..COUNT).flat_map(|i| i.to_le_bytes()).collect();
    queue.write_buffer(&storage, 0, &initial);

    let kernel = r#"
@group(0) @binding(0) var<storage, read_write> data : array<u32>;
@compute @workgroup_size(64, 1, 1)
fn cs_main(@builtin(global_invocation_id) gid : vec3<u32>) {
    if (gid.x < arrayLength(&data)) {
        data[gid.x] = data[gid.x] + 1000u;
    }
}
"#;
    let module = ShaderModule::new(
        &device,
        &ShaderModuleDesc {
            label: "addk.kernel",
            wgsl: kernel,
        },
    );
    let pipeline = ComputePipeline::new(
        &device,
        &ComputePipelineDesc {
            label: "addk.pipeline",
            layout: None,
            module: &module,
            entry_point: "cs_main",
        },
    );
    let layout = pipeline.bind_group_layout(0);
    let bind_group = BindGroup::new(
        &device,
        &BindGroupDesc {
            label: "addk.bindgroup",
            layout: &layout,
            entries: &[BindGroupEntry {
                binding: 0,
                resource: BindingResource::Buffer(&storage),
            }],
        },
    );

    let mut encoder = CommandEncoder::new(&device, "addk.encoder");
    {
        let mut cpass = encoder.begin_compute_pass("addk.pass");
        cpass.set_pipeline(&pipeline);
        cpass.set_bind_group(0, &bind_group);
        cpass.dispatch_workgroups(1, 1, 1);
    }
    encoder.copy_buffer_to_buffer(&storage, 0, &readback, 0, size);
    let _ = queue.submit(encoder);

    let bytes = readback.read_back().expect("readback");
    let result: Vec<u32> = bytes
        .chunks_exact(4)
        .map(|c| u32::from_le_bytes([c[0], c[1], c[2], c[3]]))
        .collect();
    for (i, v) in result.iter().enumerate() {
        assert_eq!(
            *v,
            (i as u32) + 1000,
            "slot {i}: write_buffer + kernel mismatch"
        );
    }
}

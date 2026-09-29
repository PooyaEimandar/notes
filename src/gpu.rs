//! Pipelines and buffers. Notes, stars and links are each one instanced draw.

use bytemuck::{Pod, Zeroable};
use sib::render::{RenderContext, bind_group, buffer, render_pass, shader, wgpu};

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Pod, Zeroable)]
pub struct Globals {
    pub view_projection: [[f32; 4]; 4],
    pub viewport: [f32; 4],
    pub time: [f32; 4],
    pub lens: [f32; 4],
    pub floor: [f32; 4],
    pub grid: [f32; 4],
    pub accent: [f32; 4],
    pub centroid: [f32; 4],
    pub detail: [f32; 4],
}

/// Three vectors per instance. `scene.wgsl` says what each one holds.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Pod, Zeroable)]
pub struct Instance {
    pub a: [f32; 4],
    pub b: [f32; 4],
    pub c: [f32; 4],
}

impl Instance {
    const ATTRIBUTES: [wgpu::VertexAttribute; 3] =
        wgpu::vertex_attr_array![0 => Float32x4, 1 => Float32x4, 2 => Float32x4];

    fn layout() -> wgpu::VertexBufferLayout<'static> {
        wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<Self>() as wgpu::BufferAddress,
            step_mode: wgpu::VertexStepMode::Instance,
            attributes: &Self::ATTRIBUTES,
        }
    }
}

#[derive(Default)]
struct Instances {
    buffer: Option<wgpu::Buffer>,
    count: u32,
}

impl Instances {
    fn set(&mut self, context: &RenderContext, label: &'static str, instances: &[Instance]) {
        let fits = self.count as usize == instances.len();
        match (&self.buffer, fits) {
            (Some(existing), true) if !instances.is_empty() => {
                context
                    .queue
                    .write_buffer(existing, 0, bytemuck::cast_slice(instances));
            }
            _ => {
                self.buffer = (!instances.is_empty()).then(|| {
                    buffer::buffer_from_data(
                        &context.device,
                        label,
                        instances,
                        wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                    )
                });
            }
        }
        self.count = instances.len() as u32;
    }

    fn draw(&self, pass: &mut wgpu::RenderPass<'_>) {
        if let Some(instances) = &self.buffer {
            pass.set_vertex_buffer(0, instances.slice(..));
            pass.draw(0..6, 0..self.count);
        }
    }
}

pub struct Gpu {
    globals: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
    orb_pipeline: wgpu::RenderPipeline,
    link_pipeline: wgpu::RenderPipeline,
    floor_pipeline: wgpu::RenderPipeline,
    orbs: Instances,
    stars: Instances,
    links: Instances,
}

impl Gpu {
    pub fn new(context: &RenderContext) -> Self {
        let device = &context.device;
        let module = shader::wgsl_module(device, "notes scene shader", include_str!("scene.wgsl"));
        let layout = bind_group::uniform_layout(
            device,
            "notes globals layout",
            wgpu::ShaderStages::VERTEX_FRAGMENT,
        );
        let globals = buffer::uniform_buffer(device, "notes globals", &Globals::default());
        let bind_group = bind_group::uniform_bind_group(device, "notes globals", &layout, &globals);
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("notes pipeline layout"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });

        let additive = wgpu::BlendComponent {
            src_factor: wgpu::BlendFactor::One,
            dst_factor: wgpu::BlendFactor::One,
            operation: wgpu::BlendOperation::Add,
        };
        let format = context.surface_config.format;
        let pipeline = |label: &'static str,
                        vertex: &'static str,
                        fragment: &'static str,
                        buffers: &[Option<wgpu::VertexBufferLayout<'static>>]| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(label),
                layout: Some(&pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &module,
                    entry_point: Some(vertex),
                    compilation_options: Default::default(),
                    buffers,
                },
                fragment: Some(wgpu::FragmentState {
                    module: &module,
                    entry_point: Some(fragment),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format,
                        blend: Some(wgpu::BlendState {
                            color: additive,
                            alpha: additive,
                        }),
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                primitive: wgpu::PrimitiveState {
                    cull_mode: None,
                    ..Default::default()
                },
                depth_stencil: None,
                multisample: wgpu::MultisampleState::default(),
                multiview_mask: None,
                cache: None,
            })
        };

        Self {
            orb_pipeline: pipeline("notes orbs", "vs_orb", "fs_orb", &[Some(Instance::layout())]),
            link_pipeline: pipeline("notes links", "vs_link", "fs_link", &[Some(Instance::layout())]),
            floor_pipeline: pipeline("notes floor", "vs_floor", "fs_floor", &[]),
            globals,
            bind_group,
            orbs: Instances::default(),
            stars: Instances::default(),
            links: Instances::default(),
        }
    }

    pub fn set_globals(&self, context: &RenderContext, globals: &Globals) {
        context
            .queue
            .write_buffer(&self.globals, 0, bytemuck::bytes_of(globals));
    }

    pub fn set_orbs(&mut self, context: &RenderContext, instances: &[Instance]) {
        self.orbs.set(context, "notes orbs", instances);
    }

    pub fn set_stars(&mut self, context: &RenderContext, instances: &[Instance]) {
        self.stars.set(context, "notes stars", instances);
    }

    pub fn set_links(&mut self, context: &RenderContext, instances: &[Instance]) {
        self.links.set(context, "notes links", instances);
    }

    /// Clears the frame and, unless `empty`, draws the scene.
    pub fn draw(
        &self,
        view: &wgpu::TextureView,
        encoder: &mut wgpu::CommandEncoder,
        clear: wgpu::Color,
        empty: bool,
    ) {
        let mut pass = render_pass::begin_color_depth(encoder, "notes scene", view, None, clear, 1.0);
        if empty {
            return;
        }
        pass.set_bind_group(0, &self.bind_group, &[]);

        pass.set_pipeline(&self.floor_pipeline);
        pass.draw(0..6, 0..1);

        pass.set_pipeline(&self.link_pipeline);
        self.links.draw(&mut pass);

        pass.set_pipeline(&self.orb_pipeline);
        self.stars.draw(&mut pass);
        self.orbs.draw(&mut pass);
    }
}

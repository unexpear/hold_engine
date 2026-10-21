//! Depth target, host vertex buffers, and the demo graphics pipelines.
use std::ffi::c_void;
use std::ptr::null_mut;

use crate::platform::ffi as win32;

use super::ffi::*;
use super::shaders;
use super::Vk;

pub(super) struct HostBuf {
    pub buffer: VkBuffer,
    pub memory: VkDeviceMemory,
    pub ptr: *mut u8,
    pub cap: u64,
}

pub(super) fn find_memory_type(
    props: &VkPhysicalDeviceMemoryProperties,
    type_bits: u32,
    want: u32,
) -> Option<u32> {
    let mut i = 0u32;
    while i < props.memoryTypeCount {
        let flags = props.memoryTypes[i as usize].propertyFlags;
        if type_bits & (1 << i) != 0 && flags & want == want {
            return Some(i);
        }
        i += 1;
    }
    None
}

pub(super) unsafe fn pick_depth_format(vk: &Vk, phys: VkPhysicalDevice) -> VkFormat {
    const CANDIDATES: [VkFormat; 3] = [
        VK_FORMAT_D32_SFLOAT,
        VK_FORMAT_D24_UNORM_S8_UINT,
        VK_FORMAT_D16_UNORM,
    ];
    for fmt in CANDIDATES {
        let mut props = std::mem::zeroed::<VkFormatProperties>();
        (vk.get_physical_device_format_props)(phys, fmt, &mut props);
        if props.optimalTilingFeatures & VK_FORMAT_FEATURE_DEPTH_STENCIL_ATTACHMENT_BIT != 0 {
            return fmt;
        }
    }
    panic!("No Vulkan depth attachment format");
}

pub(super) unsafe fn create_depth(
    vk: &Vk,
    dev: VkDevice,
    mem_props: &VkPhysicalDeviceMemoryProperties,
    format: VkFormat,
    extent: VkExtent2D,
) -> (VkImage, VkDeviceMemory, VkImageView) {
    let ci = VkImageCreateInfo {
        sType: VK_STRUCTURE_TYPE_IMAGE_CREATE_INFO,
        pNext: null_mut(),
        flags: 0,
        imageType: VK_IMAGE_TYPE_2D,
        format,
        extent: VkExtent3D {
            width: extent.width.max(1),
            height: extent.height.max(1),
            depth: 1,
        },
        mipLevels: 1,
        arrayLayers: 1,
        samples: VK_SAMPLE_COUNT_1_BIT,
        tiling: VK_IMAGE_TILING_OPTIMAL,
        usage: VK_IMAGE_USAGE_DEPTH_STENCIL_ATTACHMENT_BIT,
        sharingMode: VK_SHARING_MODE_EXCLUSIVE,
        queueFamilyIndexCount: 0,
        pQueueFamilyIndices: null_mut(),
        initialLayout: VK_IMAGE_LAYOUT_UNDEFINED,
    };
    let mut image: VkImage = 0;
    let r = (vk.create_image)(dev, &ci, null_mut(), &mut image);
    assert_eq!(r, VK_SUCCESS, "vkCreateImage (depth) failed: {}", r);

    let mut req = std::mem::zeroed::<VkMemoryRequirements>();
    (vk.get_image_memory_requirements)(dev, image, &mut req);
    let type_index = find_memory_type(
        mem_props,
        req.memoryTypeBits,
        VK_MEMORY_PROPERTY_DEVICE_LOCAL_BIT,
    )
    .or_else(|| find_memory_type(mem_props, req.memoryTypeBits, 0))
    .expect("No memory type for depth image");
    let alloc = VkMemoryAllocateInfo {
        sType: VK_STRUCTURE_TYPE_MEMORY_ALLOCATE_INFO,
        pNext: null_mut(),
        allocationSize: req.size,
        memoryTypeIndex: type_index,
    };
    let mut memory: VkDeviceMemory = 0;
    let r = (vk.allocate_memory)(dev, &alloc, null_mut(), &mut memory);
    assert_eq!(r, VK_SUCCESS, "vkAllocateMemory (depth) failed: {}", r);
    let r = (vk.bind_image_memory)(dev, image, memory, 0);
    assert_eq!(r, VK_SUCCESS, "vkBindImageMemory (depth) failed: {}", r);

    let view_ci = VkImageViewCreateInfo {
        sType: VK_STRUCTURE_TYPE_IMAGE_VIEW_CREATE_INFO,
        pNext: null_mut(),
        flags: 0,
        image,
        viewType: VK_IMAGE_VIEW_TYPE_2D,
        format,
        components: VkComponentMapping {
            r: VK_COMPONENT_SWIZZLE_IDENTITY,
            g: VK_COMPONENT_SWIZZLE_IDENTITY,
            b: VK_COMPONENT_SWIZZLE_IDENTITY,
            a: VK_COMPONENT_SWIZZLE_IDENTITY,
        },
        subresourceRange: VkImageSubresourceRange {
            aspectMask: VK_IMAGE_ASPECT_DEPTH_BIT,
            baseMipLevel: 0,
            levelCount: 1,
            baseArrayLayer: 0,
            layerCount: 1,
        },
    };
    let mut view: VkImageView = 0;
    let r = (vk.create_image_view)(dev, &view_ci, null_mut(), &mut view);
    assert_eq!(r, VK_SUCCESS, "vkCreateImageView (depth) failed: {}", r);
    (image, memory, view)
}

pub(super) unsafe fn create_host_buffer(
    vk: &Vk,
    dev: VkDevice,
    mem_props: &VkPhysicalDeviceMemoryProperties,
    size: u64,
) -> HostBuf {
    let ci = VkBufferCreateInfo {
        sType: VK_STRUCTURE_TYPE_BUFFER_CREATE_INFO,
        pNext: null_mut(),
        flags: 0,
        size,
        usage: VK_BUFFER_USAGE_VERTEX_BUFFER_BIT,
        sharingMode: VK_SHARING_MODE_EXCLUSIVE,
        queueFamilyIndexCount: 0,
        pQueueFamilyIndices: null_mut(),
    };
    let mut buffer: VkBuffer = 0;
    let r = (vk.create_buffer)(dev, &ci, null_mut(), &mut buffer);
    assert_eq!(r, VK_SUCCESS, "vkCreateBuffer failed: {}", r);

    let mut req = std::mem::zeroed::<VkMemoryRequirements>();
    (vk.get_buffer_memory_requirements)(dev, buffer, &mut req);
    let flags = VK_MEMORY_PROPERTY_HOST_VISIBLE_BIT | VK_MEMORY_PROPERTY_HOST_COHERENT_BIT;
    let type_index = find_memory_type(mem_props, req.memoryTypeBits, flags)
        .expect("No host-visible coherent memory for vertex buffer");
    let alloc = VkMemoryAllocateInfo {
        sType: VK_STRUCTURE_TYPE_MEMORY_ALLOCATE_INFO,
        pNext: null_mut(),
        allocationSize: req.size,
        memoryTypeIndex: type_index,
    };
    let mut memory: VkDeviceMemory = 0;
    let r = (vk.allocate_memory)(dev, &alloc, null_mut(), &mut memory);
    assert_eq!(r, VK_SUCCESS, "vkAllocateMemory (vertex) failed: {}", r);
    let r = (vk.bind_buffer_memory)(dev, buffer, memory, 0);
    assert_eq!(r, VK_SUCCESS, "vkBindBufferMemory failed: {}", r);

    let mut mapped: *mut c_void = null_mut();
    let r = (vk.map_memory)(dev, memory, 0, u64::MAX, 0, &mut mapped);
    assert_eq!(r, VK_SUCCESS, "vkMapMemory failed: {}", r);
    HostBuf {
        buffer,
        memory,
        ptr: mapped as *mut u8,
        cap: size,
    }
}

pub(super) unsafe fn create_demo_pipelines(
    vk: &Vk,
    dev: VkDevice,
    render_pass: VkRenderPass,
) -> (VkPipelineLayout, VkPipeline, VkPipeline) {
    let vs = shader_module(vk, dev, &shaders::TRI_VERT_SPV);
    let fs = shader_module(vk, dev, &shaders::TRI_FRAG_SPV);
    let ls = shader_module(vk, dev, &shaders::LINE_VERT_SPV);

    let push = VkPushConstantRange {
        stageFlags: VK_SHADER_STAGE_VERTEX_BIT,
        offset: 0,
        size: std::mem::size_of::<[[f32; 4]; 4]>() as u32,
    };
    let layout_ci = VkPipelineLayoutCreateInfo {
        sType: VK_STRUCTURE_TYPE_PIPELINE_LAYOUT_CREATE_INFO,
        pNext: null_mut(),
        flags: 0,
        setLayoutCount: 0,
        pSetLayouts: null_mut(),
        pushConstantRangeCount: 1,
        pPushConstantRanges: &push,
    };
    let mut layout: VkPipelineLayout = 0;
    let r = (vk.create_pipeline_layout)(dev, &layout_ci, null_mut(), &mut layout);
    assert_eq!(r, VK_SUCCESS, "vkCreatePipelineLayout failed: {}", r);

    let binding = VkVertexInputBindingDescription {
        binding: 0,
        stride: 24,
        inputRate: 0,
    };
    let attrs = [
        VkVertexInputAttributeDescription {
            location: 0,
            binding: 0,
            format: VK_FORMAT_R32G32B32_SFLOAT,
            offset: 0,
        },
        VkVertexInputAttributeDescription {
            location: 1,
            binding: 0,
            format: VK_FORMAT_R32G32B32_SFLOAT,
            offset: 12,
        },
    ];
    let vertex_input = VkPipelineVertexInputStateCreateInfo {
        sType: VK_STRUCTURE_TYPE_PIPELINE_VERTEX_INPUT_STATE_CREATE_INFO,
        pNext: null_mut(),
        flags: 0,
        vertexBindingDescriptionCount: 1,
        pVertexBindingDescriptions: &binding,
        vertexAttributeDescriptionCount: 2,
        pVertexAttributeDescriptions: attrs.as_ptr(),
    };
    let viewport = VkPipelineViewportStateCreateInfo {
        sType: VK_STRUCTURE_TYPE_PIPELINE_VIEWPORT_STATE_CREATE_INFO,
        pNext: null_mut(),
        flags: 0,
        viewportCount: 1,
        pViewports: std::ptr::null(),
        scissorCount: 1,
        pScissors: std::ptr::null(),
    };
    let dynamics = [VK_DYNAMIC_STATE_VIEWPORT, VK_DYNAMIC_STATE_SCISSOR];
    let dynamic = VkPipelineDynamicStateCreateInfo {
        sType: VK_STRUCTURE_TYPE_PIPELINE_DYNAMIC_STATE_CREATE_INFO,
        pNext: null_mut(),
        flags: 0,
        dynamicStateCount: dynamics.len() as u32,
        pDynamicStates: dynamics.as_ptr(),
    };
    let raster = VkPipelineRasterizationStateCreateInfo {
        sType: VK_STRUCTURE_TYPE_PIPELINE_RASTERIZATION_STATE_CREATE_INFO,
        pNext: null_mut(),
        flags: 0,
        depthClampEnable: VK_FALSE,
        rasterizerDiscardEnable: VK_FALSE,
        polygonMode: VK_POLYGON_MODE_FILL,
        // Software rasterizer keeps screen-space CCW, which is clockwise in Vulkan window space
        // after the clip-space Y flip. Cull the other winding so back faces match.
        cullMode: VK_CULL_MODE_BACK_BIT,
        frontFace: VK_FRONT_FACE_CLOCKWISE,
        depthBiasEnable: VK_FALSE,
        depthBiasConstantFactor: 0.0,
        depthBiasClamp: 0.0,
        depthBiasSlopeFactor: 0.0,
        lineWidth: 1.0,
    };
    let multisample = VkPipelineMultisampleStateCreateInfo {
        sType: VK_STRUCTURE_TYPE_PIPELINE_MULTISAMPLE_STATE_CREATE_INFO,
        pNext: null_mut(),
        flags: 0,
        rasterizationSamples: VK_SAMPLE_COUNT_1_BIT,
        sampleShadingEnable: VK_FALSE,
        minSampleShading: 0.0,
        pSampleMask: std::ptr::null(),
        alphaToCoverageEnable: VK_FALSE,
        alphaToOneEnable: VK_FALSE,
    };
    let stencil = std::mem::zeroed::<VkStencilOpState>();
    let depth_on = depth_state(VK_TRUE, VK_TRUE, stencil);
    let depth_off = depth_state(VK_FALSE, VK_FALSE, stencil);
    let blend_att = VkPipelineColorBlendAttachmentState {
        blendEnable: VK_FALSE,
        srcColorBlendFactor: 0,
        dstColorBlendFactor: 0,
        colorBlendOp: 0,
        srcAlphaBlendFactor: 0,
        dstAlphaBlendFactor: 0,
        alphaBlendOp: 0,
        colorWriteMask: 0xF,
    };
    let blend = VkPipelineColorBlendStateCreateInfo {
        sType: VK_STRUCTURE_TYPE_PIPELINE_COLOR_BLEND_STATE_CREATE_INFO,
        pNext: null_mut(),
        flags: 0,
        logicOpEnable: VK_FALSE,
        logicOp: 0,
        attachmentCount: 1,
        pAttachments: &blend_att,
        blendConstants: [0.0; 4],
    };
    let entry = win32::cstr("main");
    let tri_stages = [
        stage(VK_SHADER_STAGE_VERTEX_BIT, vs, entry.as_ptr()),
        stage(VK_SHADER_STAGE_FRAGMENT_BIT, fs, entry.as_ptr()),
    ];
    let line_stages = [
        stage(VK_SHADER_STAGE_VERTEX_BIT, ls, entry.as_ptr()),
        stage(VK_SHADER_STAGE_FRAGMENT_BIT, fs, entry.as_ptr()),
    ];
    let tri_ia = assembly(VK_PRIMITIVE_TOPOLOGY_TRIANGLE_LIST);
    let line_ia = assembly(VK_PRIMITIVE_TOPOLOGY_LINE_LIST);

    let tri = graphics_pipeline(
        vk,
        dev,
        layout,
        render_pass,
        &tri_stages,
        &vertex_input,
        &tri_ia,
        &viewport,
        &raster,
        &multisample,
        &depth_on,
        &blend,
        &dynamic,
    );
    let line = graphics_pipeline(
        vk,
        dev,
        layout,
        render_pass,
        &line_stages,
        &vertex_input,
        &line_ia,
        &viewport,
        &raster,
        &multisample,
        &depth_off,
        &blend,
        &dynamic,
    );

    (vk.destroy_shader_module)(dev, vs, null_mut());
    (vk.destroy_shader_module)(dev, fs, null_mut());
    (vk.destroy_shader_module)(dev, ls, null_mut());
    (layout, tri, line)
}

fn stage(
    stage: VkShaderStageFlags,
    module: VkShaderModule,
    name: *const i8,
) -> VkPipelineShaderStageCreateInfo {
    VkPipelineShaderStageCreateInfo {
        sType: VK_STRUCTURE_TYPE_PIPELINE_SHADER_STAGE_CREATE_INFO,
        pNext: null_mut(),
        flags: 0,
        stage,
        module,
        pName: name,
        pSpecializationInfo: std::ptr::null(),
    }
}

fn assembly(topology: i32) -> VkPipelineInputAssemblyStateCreateInfo {
    VkPipelineInputAssemblyStateCreateInfo {
        sType: VK_STRUCTURE_TYPE_PIPELINE_INPUT_ASSEMBLY_STATE_CREATE_INFO,
        pNext: null_mut(),
        flags: 0,
        topology,
        primitiveRestartEnable: VK_FALSE,
    }
}

fn depth_state(
    test: VkBool32,
    write: VkBool32,
    stencil: VkStencilOpState,
) -> VkPipelineDepthStencilStateCreateInfo {
    VkPipelineDepthStencilStateCreateInfo {
        sType: VK_STRUCTURE_TYPE_PIPELINE_DEPTH_STENCIL_STATE_CREATE_INFO,
        pNext: null_mut(),
        flags: 0,
        depthTestEnable: test,
        depthWriteEnable: write,
        depthCompareOp: VK_COMPARE_OP_LESS,
        depthBoundsTestEnable: VK_FALSE,
        stencilTestEnable: VK_FALSE,
        front: stencil,
        back: stencil,
        minDepthBounds: 0.0,
        maxDepthBounds: 1.0,
    }
}

unsafe fn shader_module(vk: &Vk, dev: VkDevice, code: &[u32]) -> VkShaderModule {
    let ci = VkShaderModuleCreateInfo {
        sType: VK_STRUCTURE_TYPE_SHADER_MODULE_CREATE_INFO,
        pNext: null_mut(),
        flags: 0,
        codeSize: code.len() * 4,
        pCode: code.as_ptr(),
    };
    let mut module: VkShaderModule = 0;
    let r = (vk.create_shader_module)(dev, &ci, null_mut(), &mut module);
    assert_eq!(r, VK_SUCCESS, "vkCreateShaderModule failed: {}", r);
    module
}

#[allow(clippy::too_many_arguments)]
unsafe fn graphics_pipeline(
    vk: &Vk,
    dev: VkDevice,
    layout: VkPipelineLayout,
    render_pass: VkRenderPass,
    stages: &[VkPipelineShaderStageCreateInfo],
    vertex_input: &VkPipelineVertexInputStateCreateInfo,
    input_assembly: &VkPipelineInputAssemblyStateCreateInfo,
    viewport: &VkPipelineViewportStateCreateInfo,
    raster: &VkPipelineRasterizationStateCreateInfo,
    multisample: &VkPipelineMultisampleStateCreateInfo,
    depth: &VkPipelineDepthStencilStateCreateInfo,
    blend: &VkPipelineColorBlendStateCreateInfo,
    dynamic: &VkPipelineDynamicStateCreateInfo,
) -> VkPipeline {
    let ci = VkGraphicsPipelineCreateInfo {
        sType: VK_STRUCTURE_TYPE_GRAPHICS_PIPELINE_CREATE_INFO,
        pNext: null_mut(),
        flags: 0,
        stageCount: stages.len() as u32,
        pStages: stages.as_ptr(),
        pVertexInputState: vertex_input,
        pInputAssemblyState: input_assembly,
        pTessellationState: std::ptr::null(),
        pViewportState: viewport,
        pRasterizationState: raster,
        pMultisampleState: multisample,
        pDepthStencilState: depth,
        pColorBlendState: blend,
        pDynamicState: dynamic,
        layout,
        renderPass: render_pass,
        subpass: 0,
        basePipelineHandle: 0,
        basePipelineIndex: -1,
    };
    let mut pipeline: VkPipeline = 0;
    let r = (vk.create_graphics_pipelines)(dev, 0, 1, &ci, null_mut(), &mut pipeline);
    assert_eq!(r, VK_SUCCESS, "vkCreateGraphicsPipelines failed: {}", r);
    pipeline
}

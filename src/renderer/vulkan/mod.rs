#![allow(dead_code)]
//! Vulkan renderer backend.
//!
//! Loads vulkan-1.dll at runtime (no linking, no ash/vulkano).
//! Presents the same immediate-mode demo as the software backend:
//! Gouraud triangles with a depth buffer, plus screen-space lines.

pub mod ffi;
mod geom;
mod shaders;

use std::ffi::c_void;
use std::ptr::null_mut;
use ffi::*;
use crate::platform::ffi as win32;
use crate::math::{Mat4, Vec3};
use super::{Color, Draw3d, Renderer};

const FRAMES_IN_FLIGHT: usize = 2;

// ── Vulkan function table ─────────────────────────────────────────────────────

struct Vk {
    // Global
    get_instance_proc_addr: PFN_vkGetInstanceProcAddr,
    create_instance:        PFN_vkCreateInstance,
    // Instance
    destroy_instance:            PFN_vkDestroyInstance,
    enumerate_physical_devices:  PFN_vkEnumeratePhysicalDevices,
    get_physical_device_props:   PFN_vkGetPhysicalDeviceProperties,
    get_queue_family_props:      PFN_vkGetPhysicalDeviceQueueFamilyProperties,
    create_device:               PFN_vkCreateDevice,
    create_win32_surface:        PFN_vkCreateWin32SurfaceKHR,
    destroy_surface:             PFN_vkDestroySurfaceKHR,
    get_surface_support:         PFN_vkGetPhysicalDeviceSurfaceSupportKHR,
    get_surface_caps:            PFN_vkGetPhysicalDeviceSurfaceCapabilitiesKHR,
    get_surface_formats:         PFN_vkGetPhysicalDeviceSurfaceFormatsKHR,
    get_surface_present_modes:   PFN_vkGetPhysicalDeviceSurfacePresentModesKHR,
    // Device
    destroy_device:           PFN_vkDestroyDevice,
    get_device_queue:         PFN_vkGetDeviceQueue,
    device_wait_idle:         PFN_vkDeviceWaitIdle,
    create_swapchain:         PFN_vkCreateSwapchainKHR,
    destroy_swapchain:        PFN_vkDestroySwapchainKHR,
    get_swapchain_images:     PFN_vkGetSwapchainImagesKHR,
    create_image_view:        PFN_vkCreateImageView,
    destroy_image_view:       PFN_vkDestroyImageView,
    create_render_pass:       PFN_vkCreateRenderPass,
    destroy_render_pass:      PFN_vkDestroyRenderPass,
    create_framebuffer:       PFN_vkCreateFramebuffer,
    destroy_framebuffer:      PFN_vkDestroyFramebuffer,
    create_command_pool:      PFN_vkCreateCommandPool,
    destroy_command_pool:     PFN_vkDestroyCommandPool,
    allocate_command_buffers: PFN_vkAllocateCommandBuffers,
    begin_command_buffer:     PFN_vkBeginCommandBuffer,
    end_command_buffer:       PFN_vkEndCommandBuffer,
    reset_command_buffer:     PFN_vkResetCommandBuffer,
    cmd_begin_render_pass:    PFN_vkCmdBeginRenderPass,
    cmd_end_render_pass:      PFN_vkCmdEndRenderPass,
    create_semaphore:         PFN_vkCreateSemaphore,
    destroy_semaphore:        PFN_vkDestroySemaphore,
    create_fence:             PFN_vkCreateFence,
    destroy_fence:            PFN_vkDestroyFence,
    wait_for_fences:          PFN_vkWaitForFences,
    reset_fences:             PFN_vkResetFences,
    acquire_next_image:       PFN_vkAcquireNextImageKHR,
    queue_submit:             PFN_vkQueueSubmit,
    queue_present:            PFN_vkQueuePresentKHR,
    get_physical_device_memory_props: PFN_vkGetPhysicalDeviceMemoryProperties,
    get_physical_device_format_props: PFN_vkGetPhysicalDeviceFormatProperties,
    create_shader_module:     PFN_vkCreateShaderModule,
    destroy_shader_module:    PFN_vkDestroyShaderModule,
    create_pipeline_layout:   PFN_vkCreatePipelineLayout,
    destroy_pipeline_layout:  PFN_vkDestroyPipelineLayout,
    create_graphics_pipelines: PFN_vkCreateGraphicsPipelines,
    destroy_pipeline:         PFN_vkDestroyPipeline,
    create_buffer:            PFN_vkCreateBuffer,
    destroy_buffer:           PFN_vkDestroyBuffer,
    get_buffer_memory_requirements: PFN_vkGetBufferMemoryRequirements,
    allocate_memory:          PFN_vkAllocateMemory,
    free_memory:              PFN_vkFreeMemory,
    bind_buffer_memory:       PFN_vkBindBufferMemory,
    map_memory:               PFN_vkMapMemory,
    unmap_memory:             PFN_vkUnmapMemory,
    create_image:             PFN_vkCreateImage,
    destroy_image:            PFN_vkDestroyImage,
    get_image_memory_requirements: PFN_vkGetImageMemoryRequirements,
    bind_image_memory:        PFN_vkBindImageMemory,
    cmd_bind_pipeline:        PFN_vkCmdBindPipeline,
    cmd_set_viewport:         PFN_vkCmdSetViewport,
    cmd_set_scissor:          PFN_vkCmdSetScissor,
    cmd_bind_vertex_buffers:  PFN_vkCmdBindVertexBuffers,
    cmd_push_constants:       PFN_vkCmdPushConstants,
    cmd_draw:                 PFN_vkCmdDraw,
}

// ── Sync primitives per frame-in-flight ───────────────────────────────────────

struct FrameSync {
    image_available: VkSemaphore,
    render_finished: VkSemaphore,
    in_flight:       VkFence,
    cmd_buf:         VkCommandBuffer,
    vb:              VkBuffer,
    vb_mem:          VkDeviceMemory,
    vb_ptr:          *mut u8,
    vb_cap:          u64,
}

// ── VulkanRenderer ────────────────────────────────────────────────────────────

pub struct VulkanRenderer {
    vk:              Box<Vk>,
    _lib:            win32::HMODULE,
    instance:        VkInstance,
    surface:         VkSurfaceKHR,
    physical_device: VkPhysicalDevice,
    device:          VkDevice,
    graphics_queue:  VkQueue,
    present_queue:   VkQueue,
    graphics_family: u32,
    present_family:  u32,

    swapchain:     VkSwapchainKHR,
    sc_format:     VkFormat,
    sc_extent:     VkExtent2D,
    sc_images:     Vec<VkImage>,
    sc_views:      Vec<VkImageView>,

    render_pass:   VkRenderPass,
    framebuffers:  Vec<VkFramebuffer>,

    cmd_pool:      VkCommandPool,
    frames:        Vec<FrameSync>,
    current_frame: usize,
    current_image: u32,
    pending_resize: bool,

    clear_color:   Color,

    depth_format:  VkFormat,
    depth_image:   VkImage,
    depth_memory:  VkDeviceMemory,
    depth_view:    VkImageView,
    mem_props:     VkPhysicalDeviceMemoryProperties,

    pipeline_layout: VkPipelineLayout,
    tri_pipeline:    VkPipeline,
    line_pipeline:   VkPipeline,

    vertices: Vec<GpuVertex>,
    draws:    Vec<QueuedDraw>,
}

impl VulkanRenderer {
    /// Initialise Vulkan from scratch for `hwnd`.  Panics if Vulkan isn't available.
    pub fn new(hwnd: win32::HWND, hinstance: win32::HINSTANCE, width: u32, height: u32) -> Self {
        unsafe {
            // ── Load vulkan-1.dll ─────────────────────────────────────────
            let lib_name = win32::wstr("vulkan-1.dll");
            let lib = win32::LoadLibraryW(lib_name.as_ptr());
            assert!(!lib.is_null(), "vulkan-1.dll not found — install the Vulkan runtime");

            let gipa_name = win32::cstr("vkGetInstanceProcAddr");
            let gipa_ptr = win32::GetProcAddress(lib, gipa_name.as_ptr());
            assert!(!gipa_ptr.is_null(), "vkGetInstanceProcAddr not found");
            let gipa: PFN_vkGetInstanceProcAddr = std::mem::transmute(gipa_ptr);

            // Helper closure to load a global (pre-instance) function.
            macro_rules! global {
                ($name:literal, $ty:ty) => {{
                    let n = win32::cstr($name);
                    let p = gipa(null_mut(), n.as_ptr());
                    assert!(!p.is_null(), concat!("Failed to load ", $name));
                    std::mem::transmute::<*const c_void, $ty>(p)
                }};
            }

            let create_instance: PFN_vkCreateInstance = global!("vkCreateInstance", PFN_vkCreateInstance);

            // ── Create instance ───────────────────────────────────────────
            let app_name   = win32::cstr("ForgeApp");
            let engine_name= win32::cstr("Forge");
            let app_info = VkApplicationInfo {
                sType:              VK_STRUCTURE_TYPE_APPLICATION_INFO,
                pNext:              null_mut(),
                pApplicationName:   app_name.as_ptr(),
                applicationVersion: vk_make_version(0, 1, 0),
                pEngineName:        engine_name.as_ptr(),
                engineVersion:      vk_make_version(0, 1, 0),
                apiVersion:         VK_API_VERSION_1_0,
            };

            let ext_surface      = win32::cstr("VK_KHR_surface");
            let ext_win32_surface= win32::cstr("VK_KHR_win32_surface");
            let extensions: Vec<*const i8> = vec![
                ext_surface.as_ptr(),
                ext_win32_surface.as_ptr(),
            ];

            let inst_ci = VkInstanceCreateInfo {
                sType:                   VK_STRUCTURE_TYPE_INSTANCE_CREATE_INFO,
                pNext:                   null_mut(),
                flags:                   0,
                pApplicationInfo:        &app_info,
                enabledLayerCount:       0,
                ppEnabledLayerNames:     null_mut(),
                enabledExtensionCount:   extensions.len() as u32,
                ppEnabledExtensionNames: extensions.as_ptr(),
            };

            let mut instance: VkInstance = null_mut();
            let r = create_instance(&inst_ci, null_mut(), &mut instance);
            assert_eq!(r, VK_SUCCESS, "vkCreateInstance failed: {}", r);

            // ── Load instance-level functions ─────────────────────────────
            macro_rules! inst_fn {
                ($name:literal, $ty:ty) => {{
                    let n = win32::cstr($name);
                    let p = gipa(instance, n.as_ptr());
                    assert!(!p.is_null(), concat!("Failed to load ", $name));
                    std::mem::transmute::<*const c_void, $ty>(p)
                }};
            }

            let destroy_instance:          PFN_vkDestroyInstance           = inst_fn!("vkDestroyInstance",          PFN_vkDestroyInstance);
            let enumerate_physical_devices:PFN_vkEnumeratePhysicalDevices  = inst_fn!("vkEnumeratePhysicalDevices", PFN_vkEnumeratePhysicalDevices);
            let get_physical_device_props: PFN_vkGetPhysicalDeviceProperties = inst_fn!("vkGetPhysicalDeviceProperties", PFN_vkGetPhysicalDeviceProperties);
            let get_queue_family_props:    PFN_vkGetPhysicalDeviceQueueFamilyProperties = inst_fn!("vkGetPhysicalDeviceQueueFamilyProperties", PFN_vkGetPhysicalDeviceQueueFamilyProperties);
            let create_device:             PFN_vkCreateDevice              = inst_fn!("vkCreateDevice",              PFN_vkCreateDevice);
            let create_win32_surface:      PFN_vkCreateWin32SurfaceKHR     = inst_fn!("vkCreateWin32SurfaceKHR",    PFN_vkCreateWin32SurfaceKHR);
            let destroy_surface:           PFN_vkDestroySurfaceKHR         = inst_fn!("vkDestroySurfaceKHR",        PFN_vkDestroySurfaceKHR);
            let get_surface_support:       PFN_vkGetPhysicalDeviceSurfaceSupportKHR = inst_fn!("vkGetPhysicalDeviceSurfaceSupportKHR", PFN_vkGetPhysicalDeviceSurfaceSupportKHR);
            let get_surface_caps:          PFN_vkGetPhysicalDeviceSurfaceCapabilitiesKHR = inst_fn!("vkGetPhysicalDeviceSurfaceCapabilitiesKHR", PFN_vkGetPhysicalDeviceSurfaceCapabilitiesKHR);
            let get_surface_formats:       PFN_vkGetPhysicalDeviceSurfaceFormatsKHR = inst_fn!("vkGetPhysicalDeviceSurfaceFormatsKHR", PFN_vkGetPhysicalDeviceSurfaceFormatsKHR);
            let get_surface_present_modes: PFN_vkGetPhysicalDeviceSurfacePresentModesKHR = inst_fn!("vkGetPhysicalDeviceSurfacePresentModesKHR", PFN_vkGetPhysicalDeviceSurfacePresentModesKHR);

            // ── Create Win32 surface ──────────────────────────────────────
            let surf_ci = VkWin32SurfaceCreateInfoKHR {
                sType:     VK_STRUCTURE_TYPE_WIN32_SURFACE_CREATE_INFO_KHR,
                pNext:     null_mut(),
                flags:     0,
                hinstance: hinstance as *mut c_void,
                hwnd:      hwnd as *mut c_void,
            };
            let mut surface: VkSurfaceKHR = 0;
            let r = create_win32_surface(instance, &surf_ci, null_mut(), &mut surface);
            assert_eq!(r, VK_SUCCESS, "vkCreateWin32SurfaceKHR failed: {}", r);

            // ── Pick physical device ──────────────────────────────────────
            let mut count = 0u32;
            enumerate_physical_devices(instance, &mut count, null_mut());
            assert!(count > 0, "No Vulkan physical devices found");
            let mut phys_devices = vec![null_mut::<c_void>(); count as usize];
            enumerate_physical_devices(instance, &mut count, phys_devices.as_mut_ptr());
            let physical_device = phys_devices[0]; // just take the first

            // ── Find queue families ───────────────────────────────────────
            let mut qf_count = 0u32;
            get_queue_family_props(physical_device, &mut qf_count, null_mut());
            let mut qf_props = vec![std::mem::zeroed::<VkQueueFamilyProperties>(); qf_count as usize];
            get_queue_family_props(physical_device, &mut qf_count, qf_props.as_mut_ptr());

            let mut graphics_family = u32::MAX;
            let mut present_family  = u32::MAX;
            for (i, qf) in qf_props.iter().enumerate() {
                if qf.queueFlags & VK_QUEUE_GRAPHICS_BIT != 0 {
                    graphics_family = i as u32;
                }
                let mut supported: VkBool32 = VK_FALSE;
                get_surface_support(physical_device, i as u32, surface, &mut supported);
                if supported == VK_TRUE { present_family = i as u32; }
            }
            assert_ne!(graphics_family, u32::MAX, "No graphics queue family");
            assert_ne!(present_family,  u32::MAX, "No present queue family");

            // ── Create logical device ─────────────────────────────────────
            let prio = 1.0f32;
            let mut queue_cis = vec![VkDeviceQueueCreateInfo {
                sType:            VK_STRUCTURE_TYPE_DEVICE_QUEUE_CREATE_INFO,
                pNext:            null_mut(),
                flags:            0,
                queueFamilyIndex: graphics_family,
                queueCount:       1,
                pQueuePriorities: &prio,
            }];
            if present_family != graphics_family {
                queue_cis.push(VkDeviceQueueCreateInfo {
                    sType:            VK_STRUCTURE_TYPE_DEVICE_QUEUE_CREATE_INFO,
                    pNext:            null_mut(),
                    flags:            0,
                    queueFamilyIndex: present_family,
                    queueCount:       1,
                    pQueuePriorities: &prio,
                });
            }

            let dev_ext = win32::cstr("VK_KHR_swapchain");
            let dev_extensions: Vec<*const i8> = vec![dev_ext.as_ptr()];
            let features = VkPhysicalDeviceFeatures::zeroed();
            let dev_ci = VkDeviceCreateInfo {
                sType:                   VK_STRUCTURE_TYPE_DEVICE_CREATE_INFO,
                pNext:                   null_mut(),
                flags:                   0,
                queueCreateInfoCount:    queue_cis.len() as u32,
                pQueueCreateInfos:       queue_cis.as_ptr(),
                enabledLayerCount:       0,
                ppEnabledLayerNames:     null_mut(),
                enabledExtensionCount:   dev_extensions.len() as u32,
                ppEnabledExtensionNames: dev_extensions.as_ptr(),
                pEnabledFeatures:        &features,
            };
            let mut device: VkDevice = null_mut();
            let r = create_device(physical_device, &dev_ci, null_mut(), &mut device);
            assert_eq!(r, VK_SUCCESS, "vkCreateDevice failed: {}", r);

            // ── Load device-level functions ───────────────────────────────
            // For device functions we use vkGetDeviceProcAddr via GetInstanceProcAddr trick.
            // Simplification: load them via the instance-level gipa (works for Vulkan 1.0).
            macro_rules! dev_fn {
                ($name:literal, $ty:ty) => {{
                    let n = win32::cstr($name);
                    let p = gipa(instance, n.as_ptr());
                    assert!(!p.is_null(), concat!("Failed to load ", $name));
                    std::mem::transmute::<*const c_void, $ty>(p)
                }};
            }

            let destroy_device:           PFN_vkDestroyDevice          = dev_fn!("vkDestroyDevice",           PFN_vkDestroyDevice);
            let get_device_queue:         PFN_vkGetDeviceQueue          = dev_fn!("vkGetDeviceQueue",          PFN_vkGetDeviceQueue);
            let device_wait_idle:         PFN_vkDeviceWaitIdle          = dev_fn!("vkDeviceWaitIdle",          PFN_vkDeviceWaitIdle);
            let create_swapchain:         PFN_vkCreateSwapchainKHR      = dev_fn!("vkCreateSwapchainKHR",      PFN_vkCreateSwapchainKHR);
            let destroy_swapchain:        PFN_vkDestroySwapchainKHR     = dev_fn!("vkDestroySwapchainKHR",     PFN_vkDestroySwapchainKHR);
            let get_swapchain_images:     PFN_vkGetSwapchainImagesKHR   = dev_fn!("vkGetSwapchainImagesKHR",   PFN_vkGetSwapchainImagesKHR);
            let create_image_view:        PFN_vkCreateImageView         = dev_fn!("vkCreateImageView",         PFN_vkCreateImageView);
            let destroy_image_view:       PFN_vkDestroyImageView        = dev_fn!("vkDestroyImageView",        PFN_vkDestroyImageView);
            let create_render_pass:       PFN_vkCreateRenderPass        = dev_fn!("vkCreateRenderPass",        PFN_vkCreateRenderPass);
            let destroy_render_pass:      PFN_vkDestroyRenderPass       = dev_fn!("vkDestroyRenderPass",       PFN_vkDestroyRenderPass);
            let create_framebuffer:       PFN_vkCreateFramebuffer       = dev_fn!("vkCreateFramebuffer",       PFN_vkCreateFramebuffer);
            let destroy_framebuffer:      PFN_vkDestroyFramebuffer      = dev_fn!("vkDestroyFramebuffer",      PFN_vkDestroyFramebuffer);
            let create_command_pool:      PFN_vkCreateCommandPool       = dev_fn!("vkCreateCommandPool",       PFN_vkCreateCommandPool);
            let destroy_command_pool:     PFN_vkDestroyCommandPool      = dev_fn!("vkDestroyCommandPool",      PFN_vkDestroyCommandPool);
            let allocate_command_buffers: PFN_vkAllocateCommandBuffers  = dev_fn!("vkAllocateCommandBuffers",  PFN_vkAllocateCommandBuffers);
            let begin_command_buffer:     PFN_vkBeginCommandBuffer      = dev_fn!("vkBeginCommandBuffer",      PFN_vkBeginCommandBuffer);
            let end_command_buffer:       PFN_vkEndCommandBuffer        = dev_fn!("vkEndCommandBuffer",        PFN_vkEndCommandBuffer);
            let reset_command_buffer:     PFN_vkResetCommandBuffer      = dev_fn!("vkResetCommandBuffer",      PFN_vkResetCommandBuffer);
            let cmd_begin_render_pass:    PFN_vkCmdBeginRenderPass      = dev_fn!("vkCmdBeginRenderPass",      PFN_vkCmdBeginRenderPass);
            let cmd_end_render_pass:      PFN_vkCmdEndRenderPass        = dev_fn!("vkCmdEndRenderPass",        PFN_vkCmdEndRenderPass);
            let create_semaphore:         PFN_vkCreateSemaphore         = dev_fn!("vkCreateSemaphore",         PFN_vkCreateSemaphore);
            let destroy_semaphore:        PFN_vkDestroySemaphore        = dev_fn!("vkDestroySemaphore",        PFN_vkDestroySemaphore);
            let create_fence:             PFN_vkCreateFence             = dev_fn!("vkCreateFence",             PFN_vkCreateFence);
            let destroy_fence:            PFN_vkDestroyFence            = dev_fn!("vkDestroyFence",            PFN_vkDestroyFence);
            let wait_for_fences:          PFN_vkWaitForFences           = dev_fn!("vkWaitForFences",           PFN_vkWaitForFences);
            let reset_fences:             PFN_vkResetFences             = dev_fn!("vkResetFences",             PFN_vkResetFences);
            let acquire_next_image:       PFN_vkAcquireNextImageKHR     = dev_fn!("vkAcquireNextImageKHR",     PFN_vkAcquireNextImageKHR);
            let queue_submit:             PFN_vkQueueSubmit             = dev_fn!("vkQueueSubmit",             PFN_vkQueueSubmit);
            let queue_present:            PFN_vkQueuePresentKHR         = dev_fn!("vkQueuePresentKHR",         PFN_vkQueuePresentKHR);
            let get_physical_device_memory_props: PFN_vkGetPhysicalDeviceMemoryProperties = dev_fn!("vkGetPhysicalDeviceMemoryProperties", PFN_vkGetPhysicalDeviceMemoryProperties);
            let get_physical_device_format_props: PFN_vkGetPhysicalDeviceFormatProperties = dev_fn!("vkGetPhysicalDeviceFormatProperties", PFN_vkGetPhysicalDeviceFormatProperties);
            let create_shader_module:     PFN_vkCreateShaderModule      = dev_fn!("vkCreateShaderModule",      PFN_vkCreateShaderModule);
            let destroy_shader_module:    PFN_vkDestroyShaderModule     = dev_fn!("vkDestroyShaderModule",     PFN_vkDestroyShaderModule);
            let create_pipeline_layout:   PFN_vkCreatePipelineLayout    = dev_fn!("vkCreatePipelineLayout",    PFN_vkCreatePipelineLayout);
            let destroy_pipeline_layout:  PFN_vkDestroyPipelineLayout   = dev_fn!("vkDestroyPipelineLayout",   PFN_vkDestroyPipelineLayout);
            let create_graphics_pipelines: PFN_vkCreateGraphicsPipelines = dev_fn!("vkCreateGraphicsPipelines", PFN_vkCreateGraphicsPipelines);
            let destroy_pipeline:         PFN_vkDestroyPipeline         = dev_fn!("vkDestroyPipeline",         PFN_vkDestroyPipeline);
            let create_buffer:            PFN_vkCreateBuffer            = dev_fn!("vkCreateBuffer",            PFN_vkCreateBuffer);
            let destroy_buffer:           PFN_vkDestroyBuffer           = dev_fn!("vkDestroyBuffer",           PFN_vkDestroyBuffer);
            let get_buffer_memory_requirements: PFN_vkGetBufferMemoryRequirements = dev_fn!("vkGetBufferMemoryRequirements", PFN_vkGetBufferMemoryRequirements);
            let allocate_memory:          PFN_vkAllocateMemory          = dev_fn!("vkAllocateMemory",          PFN_vkAllocateMemory);
            let free_memory:              PFN_vkFreeMemory              = dev_fn!("vkFreeMemory",              PFN_vkFreeMemory);
            let bind_buffer_memory:       PFN_vkBindBufferMemory        = dev_fn!("vkBindBufferMemory",        PFN_vkBindBufferMemory);
            let map_memory:               PFN_vkMapMemory               = dev_fn!("vkMapMemory",               PFN_vkMapMemory);
            let unmap_memory:             PFN_vkUnmapMemory             = dev_fn!("vkUnmapMemory",             PFN_vkUnmapMemory);
            let create_image:             PFN_vkCreateImage             = dev_fn!("vkCreateImage",             PFN_vkCreateImage);
            let destroy_image:            PFN_vkDestroyImage            = dev_fn!("vkDestroyImage",            PFN_vkDestroyImage);
            let get_image_memory_requirements: PFN_vkGetImageMemoryRequirements = dev_fn!("vkGetImageMemoryRequirements", PFN_vkGetImageMemoryRequirements);
            let bind_image_memory:        PFN_vkBindImageMemory         = dev_fn!("vkBindImageMemory",         PFN_vkBindImageMemory);
            let cmd_bind_pipeline:        PFN_vkCmdBindPipeline         = dev_fn!("vkCmdBindPipeline",         PFN_vkCmdBindPipeline);
            let cmd_set_viewport:         PFN_vkCmdSetViewport          = dev_fn!("vkCmdSetViewport",          PFN_vkCmdSetViewport);
            let cmd_set_scissor:          PFN_vkCmdSetScissor           = dev_fn!("vkCmdSetScissor",           PFN_vkCmdSetScissor);
            let cmd_bind_vertex_buffers:  PFN_vkCmdBindVertexBuffers    = dev_fn!("vkCmdBindVertexBuffers",    PFN_vkCmdBindVertexBuffers);
            let cmd_push_constants:       PFN_vkCmdPushConstants        = dev_fn!("vkCmdPushConstants",        PFN_vkCmdPushConstants);
            let cmd_draw:                 PFN_vkCmdDraw                 = dev_fn!("vkCmdDraw",                 PFN_vkCmdDraw);

            let vk = Box::new(Vk {
                get_instance_proc_addr: gipa,
                create_instance,
                destroy_instance,
                enumerate_physical_devices,
                get_physical_device_props,
                get_queue_family_props,
                create_device,
                create_win32_surface,
                destroy_surface,
                get_surface_support,
                get_surface_caps,
                get_surface_formats,
                get_surface_present_modes,
                destroy_device,
                get_device_queue,
                device_wait_idle,
                create_swapchain,
                destroy_swapchain,
                get_swapchain_images,
                create_image_view,
                destroy_image_view,
                create_render_pass,
                destroy_render_pass,
                create_framebuffer,
                destroy_framebuffer,
                create_command_pool,
                destroy_command_pool,
                allocate_command_buffers,
                begin_command_buffer,
                end_command_buffer,
                reset_command_buffer,
                cmd_begin_render_pass,
                cmd_end_render_pass,
                create_semaphore,
                destroy_semaphore,
                create_fence,
                destroy_fence,
                wait_for_fences,
                reset_fences,
                acquire_next_image,
                queue_submit,
                queue_present,
                get_physical_device_memory_props,
                get_physical_device_format_props,
                create_shader_module,
                destroy_shader_module,
                create_pipeline_layout,
                destroy_pipeline_layout,
                create_graphics_pipelines,
                destroy_pipeline,
                create_buffer,
                destroy_buffer,
                get_buffer_memory_requirements,
                allocate_memory,
                free_memory,
                bind_buffer_memory,
                map_memory,
                unmap_memory,
                create_image,
                destroy_image,
                get_image_memory_requirements,
                bind_image_memory,
                cmd_bind_pipeline,
                cmd_set_viewport,
                cmd_set_scissor,
                cmd_bind_vertex_buffers,
                cmd_push_constants,
                cmd_draw,
            });

            // ── Queues ────────────────────────────────────────────────────
            let mut graphics_queue: VkQueue = null_mut();
            let mut present_queue:  VkQueue = null_mut();
            (vk.get_device_queue)(device, graphics_family, 0, &mut graphics_queue);
            (vk.get_device_queue)(device, present_family,  0, &mut present_queue);

            // ── Build swapchain + depth + render pass + framebuffers ──────
            let (swapchain, sc_format, sc_extent, sc_images, sc_views) =
                create_swapchain_objects(&vk, physical_device, device, surface,
                    graphics_family, present_family, width, height);

            let mut mem_props = std::mem::zeroed::<VkPhysicalDeviceMemoryProperties>();
            (vk.get_physical_device_memory_props)(physical_device, &mut mem_props);
            let depth_format = geom::pick_depth_format(&vk, physical_device);
            let render_pass = create_render_pass_obj(&vk, device, sc_format, depth_format);
            let (depth_image, depth_memory, depth_view) =
                geom::create_depth(&vk, device, &mem_props, depth_format, sc_extent);
            let framebuffers = create_framebuffers_obj(
                &vk, device, render_pass, &sc_views, depth_view, sc_extent,
            );
            let (pipeline_layout, tri_pipeline, line_pipeline) =
                geom::create_demo_pipelines(&vk, device, render_pass);

            // ── Command pool ──────────────────────────────────────────────
            let pool_ci = VkCommandPoolCreateInfo {
                sType:            VK_STRUCTURE_TYPE_COMMAND_POOL_CREATE_INFO,
                pNext:            null_mut(),
                flags:            0x0002, // RESET_COMMAND_BUFFER_BIT
                queueFamilyIndex: graphics_family,
            };
            let mut cmd_pool: VkCommandPool = 0;
            let r = (vk.create_command_pool)(device, &pool_ci, null_mut(), &mut cmd_pool);
            assert_eq!(r, VK_SUCCESS, "vkCreateCommandPool failed");

            // ── Per-frame sync + command buffers ──────────────────────────
            let alloc_info = VkCommandBufferAllocateInfo {
                sType:              VK_STRUCTURE_TYPE_COMMAND_BUFFER_ALLOCATE_INFO,
                pNext:              null_mut(),
                commandPool:        cmd_pool,
                level:              VK_COMMAND_BUFFER_LEVEL_PRIMARY,
                commandBufferCount: FRAMES_IN_FLIGHT as u32,
            };
            let mut cmd_bufs = vec![null_mut::<c_void>(); FRAMES_IN_FLIGHT];
            let r = (vk.allocate_command_buffers)(device, &alloc_info, cmd_bufs.as_mut_ptr());
            assert_eq!(r, VK_SUCCESS, "vkAllocateCommandBuffers failed");

            let sem_ci = VkSemaphoreCreateInfo {
                sType: VK_STRUCTURE_TYPE_SEMAPHORE_CREATE_INFO,
                pNext: null_mut(), flags: 0,
            };
            let fence_ci = VkFenceCreateInfo {
                sType: VK_STRUCTURE_TYPE_FENCE_CREATE_INFO,
                pNext: null_mut(),
                flags: VK_FENCE_CREATE_SIGNALED_BIT,
            };

            let mut frames = Vec::with_capacity(FRAMES_IN_FLIGHT);
            for i in 0..FRAMES_IN_FLIGHT {
                let mut img_avail: VkSemaphore = 0;
                let mut render_done: VkSemaphore = 0;
                let mut fence: VkFence = 0;
                (vk.create_semaphore)(device, &sem_ci, null_mut(), &mut img_avail);
                (vk.create_semaphore)(device, &sem_ci, null_mut(), &mut render_done);
                (vk.create_fence)(device, &fence_ci, null_mut(), &mut fence);
                let host = geom::create_host_buffer(&vk, device, &mem_props, 256 * 24);
                frames.push(FrameSync {
                    image_available: img_avail,
                    render_finished: render_done,
                    in_flight: fence,
                    cmd_buf: cmd_bufs[i],
                    vb: host.buffer,
                    vb_mem: host.memory,
                    vb_ptr: host.ptr,
                    vb_cap: host.cap,
                });
            }

            VulkanRenderer {
                vk,
                _lib: lib,
                instance,
                surface,
                physical_device,
                device,
                graphics_queue,
                present_queue,
                graphics_family,
                present_family,
                swapchain,
                sc_format,
                sc_extent,
                sc_images,
                sc_views,
                render_pass,
                framebuffers,
                cmd_pool,
                frames,
                current_frame: 0,
                current_image: 0,
                pending_resize: false,
                clear_color: Color::BLACK,
                depth_format,
                depth_image,
                depth_memory,
                depth_view,
                mem_props,
                pipeline_layout,
                tri_pipeline,
                line_pipeline,
                vertices: Vec::new(),
                draws: Vec::new(),
            }
        }
    }

    pub fn current_command_buffer(&self) -> VkCommandBuffer {
        self.frames[self.current_frame].cmd_buf
    }

    fn recreate_swapchain(&mut self, width: u32, height: u32) {
        unsafe {
            (self.vk.device_wait_idle)(self.device);

            for &framebuffer in &self.framebuffers {
                (self.vk.destroy_framebuffer)(self.device, framebuffer, null_mut());
            }
            self.destroy_depth();
            for &view in &self.sc_views {
                (self.vk.destroy_image_view)(self.device, view, null_mut());
            }
            (self.vk.destroy_swapchain)(self.device, self.swapchain, null_mut());

            let (swapchain, format, extent, images, views) = create_swapchain_objects(
                &self.vk,
                self.physical_device,
                self.device,
                self.surface,
                self.graphics_family,
                self.present_family,
                width,
                height,
            );

            self.swapchain = swapchain;
            self.sc_format = format;
            self.sc_extent = extent;
            self.sc_images = images;
            self.sc_views = views;
            let (depth_image, depth_memory, depth_view) = geom::create_depth(
                &self.vk, self.device, &self.mem_props, self.depth_format, self.sc_extent,
            );
            self.depth_image = depth_image;
            self.depth_memory = depth_memory;
            self.depth_view = depth_view;
            self.framebuffers = create_framebuffers_obj(
                &self.vk,
                self.device,
                self.render_pass,
                &self.sc_views,
                self.depth_view,
                self.sc_extent,
            );
        }
    }

    fn acquire_swapchain_image(&mut self, semaphore: VkSemaphore) -> (u32, bool) {
        unsafe {
            loop {
                let mut image_index = 0u32;
                let result = (self.vk.acquire_next_image)(
                    self.device,
                    self.swapchain,
                    u64::MAX,
                    semaphore,
                    0,
                    &mut image_index,
                );

                match result {
                    VK_SUCCESS => return (image_index, false),
                    VK_SUBOPTIMAL_KHR => return (image_index, true),
                    VK_ERROR_OUT_OF_DATE_KHR => {
                        self.recreate_swapchain(self.sc_extent.width, self.sc_extent.height);
                    }
                    _ => panic!("vkAcquireNextImageKHR failed: {}", result),
                }
            }
        }
    }
}

impl Renderer for VulkanRenderer {
    fn width(&self)  -> u32 { self.sc_extent.width }
    fn height(&self) -> u32 { self.sc_extent.height }

    fn resize(&mut self, width: u32, height: u32) {
        self.recreate_swapchain(width, height);
        self.pending_resize = false;
    }

    fn begin_frame(&mut self) {
        self.vertices.clear();
        self.draws.clear();
        unsafe {
            let image_available = self.frames[self.current_frame].image_available;
            let in_flight = self.frames[self.current_frame].in_flight;

            // Wait for this frame slot to be free.
            (self.vk.wait_for_fences)(self.device, 1, &in_flight, VK_TRUE, u64::MAX);
            (self.vk.reset_fences)(self.device, 1, &in_flight);

            // Acquire the next swapchain image.
            let (image_index, needs_resize) = self.acquire_swapchain_image(image_available);
            self.pending_resize = needs_resize;
            self.current_image = image_index;

            // Record: begin command buffer → begin render pass.
            let f = &self.frames[self.current_frame];
            let begin_info = VkCommandBufferBeginInfo {
                sType:            VK_STRUCTURE_TYPE_COMMAND_BUFFER_BEGIN_INFO,
                pNext:            null_mut(),
                flags:            0x0001, // ONE_TIME_SUBMIT_BIT
                pInheritanceInfo: null_mut(),
            };
            (self.vk.reset_command_buffer)(f.cmd_buf, 0);
            (self.vk.begin_command_buffer)(f.cmd_buf, &begin_info);
        }
    }

    fn clear(&mut self, color: Color) {
        self.clear_color = color;
        unsafe {
            let f = &self.frames[self.current_frame];
            let clear_values = [
                VkClearValue {
                    color: VkClearColorValue {
                        float32: [color.r, color.g, color.b, color.a],
                    },
                },
                VkClearValue {
                    depthStencil: VkClearDepthStencilValue { depth: 1.0, stencil: 0 },
                },
            ];
            let rp_begin = VkRenderPassBeginInfo {
                sType:       VK_STRUCTURE_TYPE_RENDER_PASS_BEGIN_INFO,
                pNext:       null_mut(),
                renderPass:  self.render_pass,
                framebuffer: self.framebuffers[self.current_image as usize],
                renderArea:  VkRect2D {
                    offset: VkOffset2D { x: 0, y: 0 },
                    extent: self.sc_extent,
                },
                clearValueCount: clear_values.len() as u32,
                pClearValues:    clear_values.as_ptr(),
            };
            (self.vk.cmd_begin_render_pass)(f.cmd_buf, &rp_begin, 0 /*INLINE*/);
        }
    }

    fn end_frame(&mut self) {
        unsafe {
            let (image_available, render_finished, in_flight) = {
                let frame = &self.frames[self.current_frame];
                (frame.image_available, frame.render_finished, frame.in_flight)
            };

            self.flush_draws();
            let cmd_buf = self.frames[self.current_frame].cmd_buf;
            (self.vk.cmd_end_render_pass)(cmd_buf);
            (self.vk.end_command_buffer)(cmd_buf);

            let wait_stage: VkPipelineStageFlags = VK_PIPELINE_STAGE_COLOR_ATTACHMENT_OUTPUT_BIT;
            let submit = VkSubmitInfo {
                sType:                VK_STRUCTURE_TYPE_SUBMIT_INFO,
                pNext:                null_mut(),
                waitSemaphoreCount:   1,
                pWaitSemaphores:      &image_available,
                pWaitDstStageMask:    &wait_stage,
                commandBufferCount:   1,
                pCommandBuffers:      &cmd_buf,
                signalSemaphoreCount: 1,
                pSignalSemaphores:    &render_finished,
            };
            let submit_result = (self.vk.queue_submit)(self.graphics_queue, 1, &submit, in_flight);
            assert_eq!(submit_result, VK_SUCCESS, "vkQueueSubmit failed: {}", submit_result);

            let present = VkPresentInfoKHR {
                sType:              VK_STRUCTURE_TYPE_PRESENT_INFO_KHR,
                pNext:              null_mut(),
                waitSemaphoreCount: 1,
                pWaitSemaphores:    &render_finished,
                swapchainCount:     1,
                pSwapchains:        &self.swapchain,
                pImageIndices:      &self.current_image,
                pResults:           null_mut(),
            };
            let present_result = (self.vk.queue_present)(self.present_queue, &present);

            if present_result == VK_ERROR_OUT_OF_DATE_KHR || present_result == VK_SUBOPTIMAL_KHR || self.pending_resize {
                self.recreate_swapchain(self.sc_extent.width, self.sc_extent.height);
                self.pending_resize = false;
            } else {
                assert_eq!(present_result, VK_SUCCESS, "vkQueuePresentKHR failed: {}", present_result);
            }

            self.current_frame = (self.current_frame + 1) % FRAMES_IN_FLIGHT;
        }
    }
}

impl VulkanRenderer {
    fn destroy_depth(&mut self) {
        unsafe {
            (self.vk.destroy_image_view)(self.device, self.depth_view, null_mut());
            (self.vk.destroy_image)(self.device, self.depth_image, null_mut());
            (self.vk.free_memory)(self.device, self.depth_memory, null_mut());
            self.depth_view = 0;
            self.depth_image = 0;
            self.depth_memory = 0;
        }
    }

    fn ensure_vertex_buffer(&mut self, bytes: u64) {
        let (old_vb, old_mem, old_cap) = {
            let frame = &self.frames[self.current_frame];
            (frame.vb, frame.vb_mem, frame.vb_cap)
        };
        if old_cap >= bytes { return; }
        let new_cap = bytes.max(old_cap.saturating_mul(2)).max(4096);
        unsafe {
            if old_vb != 0 {
                (self.vk.unmap_memory)(self.device, old_mem);
                (self.vk.destroy_buffer)(self.device, old_vb, null_mut());
                (self.vk.free_memory)(self.device, old_mem, null_mut());
            }
            let host = geom::create_host_buffer(&self.vk, self.device, &self.mem_props, new_cap);
            let frame = &mut self.frames[self.current_frame];
            frame.vb = host.buffer;
            frame.vb_mem = host.memory;
            frame.vb_ptr = host.ptr;
            frame.vb_cap = host.cap;
        }
    }

    /// Upload queued immediate-mode draws and record them inside the open render pass.
    fn flush_draws(&mut self) {
        if self.vertices.is_empty() || self.sc_extent.width == 0 || self.sc_extent.height == 0 {
            return;
        }
        let bytes = (self.vertices.len() * std::mem::size_of::<GpuVertex>()) as u64;
        self.ensure_vertex_buffer(bytes);
        unsafe {
            let frame = &self.frames[self.current_frame];
            std::ptr::copy_nonoverlapping(
                self.vertices.as_ptr() as *const u8,
                frame.vb_ptr,
                bytes as usize,
            );
            let cmd = frame.cmd_buf;
            let vb = frame.vb;
            let viewport = VkViewport {
                x: 0.0,
                y: 0.0,
                width: self.sc_extent.width as f32,
                height: self.sc_extent.height as f32,
                minDepth: 0.0,
                maxDepth: 1.0,
            };
            let scissor = VkRect2D {
                offset: VkOffset2D { x: 0, y: 0 },
                extent: self.sc_extent,
            };
            (self.vk.cmd_set_viewport)(cmd, 0, 1, &viewport);
            (self.vk.cmd_set_scissor)(cmd, 0, 1, &scissor);
            let offset = 0u64;
            (self.vk.cmd_bind_vertex_buffers)(cmd, 0, 1, &vb, &offset);

            let mut bound_lines: Option<bool> = None;
            for draw in &self.draws {
                if bound_lines != Some(draw.lines) {
                    let pipeline = if draw.lines { self.line_pipeline } else { self.tri_pipeline };
                    (self.vk.cmd_bind_pipeline)(cmd, VK_PIPELINE_BIND_POINT_GRAPHICS, pipeline);
                    bound_lines = Some(draw.lines);
                }
                if !draw.lines {
                    (self.vk.cmd_push_constants)(
                        cmd,
                        self.pipeline_layout,
                        VK_SHADER_STAGE_VERTEX_BIT,
                        0,
                        64,
                        draw.mvp.cols.as_ptr() as *const c_void,
                    );
                }
                (self.vk.cmd_draw)(cmd, draw.count, 1, draw.first, 0);
            }
        }
    }

    fn enqueue(&mut self, verts: &[GpuVertex], lines: bool, mvp: Mat4) {
        let first = self.vertices.len() as u32;
        self.vertices.extend_from_slice(verts);
        self.draws.push(QueuedDraw { lines, first, count: verts.len() as u32, mvp });
    }
}

#[repr(C)]
#[derive(Clone, Copy)]
struct GpuVertex {
    pos: [f32; 3],
    color: [f32; 3],
}

struct QueuedDraw {
    lines: bool,
    first: u32,
    count: u32,
    mvp: Mat4,
}

impl Draw3d for VulkanRenderer {
    fn draw_triangle_3d(&mut self, v0: Vec3, v1: Vec3, v2: Vec3, color: Color, mvp: &Mat4) {
        self.draw_triangle_3d_shaded(v0, v1, v2, color, color, color, mvp);
    }

    fn draw_triangle_3d_shaded(
        &mut self,
        v0: Vec3, v1: Vec3, v2: Vec3,
        c0: Color, c1: Color, c2: Color,
        mvp: &Mat4,
    ) {
        let pack = |v: Vec3, c: Color| GpuVertex {
            pos: [v.x, v.y, v.z],
            color: [c.r, c.g, c.b],
        };
        self.enqueue(&[pack(v0, c0), pack(v1, c1), pack(v2, c2)], false, *mvp);
    }

    fn draw_line(&mut self, x0: i32, y0: i32, x1: i32, y1: i32, color: Color) {
        let w = self.sc_extent.width as f32;
        let h = self.sc_extent.height as f32;
        if w < 1.0 || h < 1.0 { return; }
        let ndc = |x: i32, y: i32| {
            let px = x as f32 + 0.5;
            let py = y as f32 + 0.5;
            [(px / w) * 2.0 - 1.0, (py / h) * 2.0 - 1.0, 0.0]
        };
        let col = [color.r, color.g, color.b];
        self.enqueue(
            &[
                GpuVertex { pos: ndc(x0, y0), color: col },
                GpuVertex { pos: ndc(x1, y1), color: col },
            ],
            true,
            Mat4::IDENTITY,
        );
    }
}

impl Drop for VulkanRenderer {
    fn drop(&mut self) {
        unsafe {
            (self.vk.device_wait_idle)(self.device);
            let vk = &self.vk;
            for f in &self.frames {
                (vk.destroy_semaphore)(self.device, f.image_available, null_mut());
                (vk.destroy_semaphore)(self.device, f.render_finished, null_mut());
                (vk.destroy_fence)(self.device, f.in_flight, null_mut());
                if f.vb != 0 {
                    (vk.unmap_memory)(self.device, f.vb_mem);
                    (vk.destroy_buffer)(self.device, f.vb, null_mut());
                    (vk.free_memory)(self.device, f.vb_mem, null_mut());
                }
            }
            (vk.destroy_pipeline)(self.device, self.tri_pipeline, null_mut());
            (vk.destroy_pipeline)(self.device, self.line_pipeline, null_mut());
            (vk.destroy_pipeline_layout)(self.device, self.pipeline_layout, null_mut());
            (vk.destroy_command_pool)(self.device, self.cmd_pool, null_mut());
            (vk.destroy_image_view)(self.device, self.depth_view, null_mut());
            (vk.destroy_image)(self.device, self.depth_image, null_mut());
            (vk.free_memory)(self.device, self.depth_memory, null_mut());
            for &fb in &self.framebuffers { (vk.destroy_framebuffer)(self.device, fb, null_mut()); }
            (vk.destroy_render_pass)(self.device, self.render_pass, null_mut());
            for &iv in &self.sc_views    { (vk.destroy_image_view)(self.device, iv, null_mut()); }
            (vk.destroy_swapchain)(self.device, self.swapchain, null_mut());
            (vk.destroy_surface)(self.instance, self.surface, null_mut());
            (vk.destroy_device)(self.device, null_mut());
            (vk.destroy_instance)(self.instance, null_mut());
            win32::FreeLibrary(self._lib);
        }
    }
}

// ── Private helper functions ──────────────────────────────────────────────────

unsafe fn create_swapchain_objects(
    vk: &Vk,
    phys: VkPhysicalDevice,
    dev:  VkDevice,
    surf: VkSurfaceKHR,
    graphics_family: u32,
    present_family:  u32,
    width: u32, height: u32,
) -> (VkSwapchainKHR, VkFormat, VkExtent2D, Vec<VkImage>, Vec<VkImageView>) {
    let mut caps = VkSurfaceCapabilitiesKHR::zeroed();
    (vk.get_surface_caps)(phys, surf, &mut caps);

    // Pick format: prefer B8G8R8A8_SRGB.
    let mut fmt_count = 0u32;
    (vk.get_surface_formats)(phys, surf, &mut fmt_count, null_mut());
    let mut formats = vec![unsafe { std::mem::zeroed::<VkSurfaceFormatKHR>() }; fmt_count as usize];
    (vk.get_surface_formats)(phys, surf, &mut fmt_count, formats.as_mut_ptr());
    let chosen_fmt = formats.iter().find(|f| f.format == VK_FORMAT_B8G8R8A8_SRGB)
        .copied().unwrap_or(formats[0]);

    // Pick present mode: prefer MAILBOX, fall back to FIFO.
    let mut pm_count = 0u32;
    (vk.get_surface_present_modes)(phys, surf, &mut pm_count, null_mut());
    let mut modes = vec![0i32; pm_count as usize];
    (vk.get_surface_present_modes)(phys, surf, &mut pm_count, modes.as_mut_ptr());
    let present_mode = if modes.contains(&VK_PRESENT_MODE_MAILBOX_KHR) {
        VK_PRESENT_MODE_MAILBOX_KHR
    } else {
        VK_PRESENT_MODE_FIFO_KHR
    };

    let extent = if caps.currentExtent.width != u32::MAX {
        caps.currentExtent
    } else {
        VkExtent2D {
            width:  width.clamp(caps.minImageExtent.width,  caps.maxImageExtent.width),
            height: height.clamp(caps.minImageExtent.height, caps.maxImageExtent.height),
        }
    };

    let image_count = (caps.minImageCount + 1).min(
        if caps.maxImageCount > 0 { caps.maxImageCount } else { u32::MAX }
    );

    let (sharing_mode, qf_count, qf_indices): (VkSharingMode, u32, [u32; 2]) =
        if graphics_family != present_family {
            (1 /*CONCURRENT*/, 2, [graphics_family, present_family])
        } else {
            (VK_SHARING_MODE_EXCLUSIVE, 0, [0, 0])
        };

    let sc_ci = VkSwapchainCreateInfoKHR {
        sType:                 VK_STRUCTURE_TYPE_SWAPCHAIN_CREATE_INFO_KHR,
        pNext:                 null_mut(),
        flags:                 0,
        surface:               surf,
        minImageCount:         image_count,
        imageFormat:           chosen_fmt.format,
        imageColorSpace:       chosen_fmt.colorSpace,
        imageExtent:           extent,
        imageArrayLayers:      1,
        imageUsage:            VK_IMAGE_USAGE_COLOR_ATTACHMENT_BIT,
        imageSharingMode:      sharing_mode,
        queueFamilyIndexCount: qf_count,
        pQueueFamilyIndices:   if qf_count > 0 { qf_indices.as_ptr() } else { null_mut() },
        preTransform:          caps.currentTransform,
        compositeAlpha:        VK_COMPOSITE_ALPHA_OPAQUE_BIT_KHR,
        presentMode:           present_mode,
        clipped:               VK_TRUE,
        oldSwapchain:          0,
    };

    let mut swapchain: VkSwapchainKHR = 0;
    let r = (vk.create_swapchain)(dev, &sc_ci, null_mut(), &mut swapchain);
    assert_eq!(r, VK_SUCCESS, "vkCreateSwapchainKHR failed: {}", r);

    let mut img_count = 0u32;
    (vk.get_swapchain_images)(dev, swapchain, &mut img_count, null_mut());
    let mut images = vec![0u64; img_count as usize];
    (vk.get_swapchain_images)(dev, swapchain, &mut img_count, images.as_mut_ptr());

    let views: Vec<VkImageView> = images.iter().map(|&img| {
        let ci = VkImageViewCreateInfo {
            sType:    VK_STRUCTURE_TYPE_IMAGE_VIEW_CREATE_INFO,
            pNext:    null_mut(),
            flags:    0,
            image:    img,
            viewType: VK_IMAGE_VIEW_TYPE_2D,
            format:   chosen_fmt.format,
            components: VkComponentMapping {
                r: VK_COMPONENT_SWIZZLE_IDENTITY, g: VK_COMPONENT_SWIZZLE_IDENTITY,
                b: VK_COMPONENT_SWIZZLE_IDENTITY, a: VK_COMPONENT_SWIZZLE_IDENTITY,
            },
            subresourceRange: VkImageSubresourceRange {
                aspectMask:     VK_IMAGE_ASPECT_COLOR_BIT,
                baseMipLevel:   0,
                levelCount:     1,
                baseArrayLayer: 0,
                layerCount:     1,
            },
        };
        let mut view: VkImageView = 0;
        (vk.create_image_view)(dev, &ci, null_mut(), &mut view);
        view
    }).collect();

    (swapchain, chosen_fmt.format, extent, images, views)
}

unsafe fn create_render_pass_obj(vk: &Vk, dev: VkDevice, format: VkFormat, depth_format: VkFormat) -> VkRenderPass {
    let attachments = [
        VkAttachmentDescription {
            flags:          0,
            format,
            samples:        VK_SAMPLE_COUNT_1_BIT,
            loadOp:         VK_ATTACHMENT_LOAD_OP_CLEAR,
            storeOp:        VK_ATTACHMENT_STORE_OP_STORE,
            stencilLoadOp:  VK_ATTACHMENT_LOAD_OP_DONT_CARE,
            stencilStoreOp: VK_ATTACHMENT_STORE_OP_DONT_CARE,
            initialLayout:  VK_IMAGE_LAYOUT_UNDEFINED,
            finalLayout:    VK_IMAGE_LAYOUT_PRESENT_SRC_KHR,
        },
        VkAttachmentDescription {
            flags:          0,
            format:         depth_format,
            samples:        VK_SAMPLE_COUNT_1_BIT,
            loadOp:         VK_ATTACHMENT_LOAD_OP_CLEAR,
            storeOp:        VK_ATTACHMENT_STORE_OP_DONT_CARE,
            stencilLoadOp:  VK_ATTACHMENT_LOAD_OP_DONT_CARE,
            stencilStoreOp: VK_ATTACHMENT_STORE_OP_DONT_CARE,
            initialLayout:  VK_IMAGE_LAYOUT_UNDEFINED,
            finalLayout:    VK_IMAGE_LAYOUT_DEPTH_STENCIL_ATTACHMENT_OPTIMAL,
        },
    ];
    let color_ref = VkAttachmentReference {
        attachment: 0,
        layout:     VK_IMAGE_LAYOUT_COLOR_ATTACHMENT_OPTIMAL,
    };
    let depth_ref = VkAttachmentReference {
        attachment: 1,
        layout:     VK_IMAGE_LAYOUT_DEPTH_STENCIL_ATTACHMENT_OPTIMAL,
    };
    let subpass = VkSubpassDescription {
        flags:                   0,
        pipelineBindPoint:       VK_PIPELINE_BIND_POINT_GRAPHICS,
        inputAttachmentCount:    0,
        pInputAttachments:       null_mut(),
        colorAttachmentCount:    1,
        pColorAttachments:       &color_ref,
        pResolveAttachments:     null_mut(),
        pDepthStencilAttachment: &depth_ref,
        preserveAttachmentCount: 0,
        pPreserveAttachments:    null_mut(),
    };
    let dependency = VkSubpassDependency {
        srcSubpass:      VK_SUBPASS_EXTERNAL,
        dstSubpass:      0,
        srcStageMask:    VK_PIPELINE_STAGE_COLOR_ATTACHMENT_OUTPUT_BIT
            | VK_PIPELINE_STAGE_EARLY_FRAGMENT_TESTS_BIT,
        dstStageMask:    VK_PIPELINE_STAGE_COLOR_ATTACHMENT_OUTPUT_BIT
            | VK_PIPELINE_STAGE_EARLY_FRAGMENT_TESTS_BIT
            | VK_PIPELINE_STAGE_LATE_FRAGMENT_TESTS_BIT,
        srcAccessMask:   0,
        dstAccessMask:   VK_ACCESS_COLOR_ATTACHMENT_WRITE_BIT
            | VK_ACCESS_DEPTH_STENCIL_ATTACHMENT_WRITE_BIT,
        dependencyFlags: 0,
    };
    let rp_ci = VkRenderPassCreateInfo {
        sType:           VK_STRUCTURE_TYPE_RENDER_PASS_CREATE_INFO,
        pNext:           null_mut(),
        flags:           0,
        attachmentCount: attachments.len() as u32,
        pAttachments:    attachments.as_ptr(),
        subpassCount:    1,
        pSubpasses:      &subpass,
        dependencyCount: 1,
        pDependencies:   &dependency,
    };
    let mut render_pass: VkRenderPass = 0;
    let r = (vk.create_render_pass)(dev, &rp_ci, null_mut(), &mut render_pass);
    assert_eq!(r, VK_SUCCESS, "vkCreateRenderPass failed: {}", r);
    render_pass
}

unsafe fn create_framebuffers_obj(
    vk:          &Vk,
    dev:         VkDevice,
    render_pass: VkRenderPass,
    views:       &[VkImageView],
    depth_view:  VkImageView,
    extent:      VkExtent2D,
) -> Vec<VkFramebuffer> {
    views.iter().map(|&view| {
        let attachments = [view, depth_view];
        let ci = VkFramebufferCreateInfo {
            sType:           VK_STRUCTURE_TYPE_FRAMEBUFFER_CREATE_INFO,
            pNext:           null_mut(),
            flags:           0,
            renderPass:      render_pass,
            attachmentCount: 2,
            pAttachments:    attachments.as_ptr(),
            width:           extent.width,
            height:          extent.height,
            layers:          1,
        };
        let mut fb: VkFramebuffer = 0;
        (vk.create_framebuffer)(dev, &ci, null_mut(), &mut fb);
        fb
    }).collect()
}

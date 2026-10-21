//! Minimal raw Vulkan FFI — loaded dynamically from vulkan-1.dll.
//! No ash, no vulkano. Function pointers are fetched at runtime.
#![allow(non_snake_case, non_camel_case_types, dead_code, clippy::upper_case_acronyms)]

use std::ffi::c_void;

// ── Dispatchable handles (pointer-sized) ──────────────────────────────────────
pub type VkInstance       = *mut c_void;
pub type VkPhysicalDevice = *mut c_void;
pub type VkDevice         = *mut c_void;
pub type VkQueue          = *mut c_void;
pub type VkCommandBuffer  = *mut c_void;

// ── Non-dispatchable handles (always 64-bit) ──────────────────────────────────
pub type VkSurfaceKHR      = u64;
pub type VkSwapchainKHR    = u64;
pub type VkImage           = u64;
pub type VkImageView       = u64;
pub type VkFramebuffer     = u64;
pub type VkRenderPass      = u64;
pub type VkCommandPool     = u64;
pub type VkSemaphore       = u64;
pub type VkFence           = u64;
pub type VkShaderModule    = u64;
pub type VkPipeline        = u64;
pub type VkPipelineLayout  = u64;
pub type VkDeviceMemory    = u64;
pub type VkBuffer          = u64;
pub type VkDescriptorPool  = u64;
pub type VkDescriptorSet   = u64;
pub type VkDescriptorSetLayout = u64;

// ── Primitive type aliases ─────────────────────────────────────────────────────
pub type VkResult          = i32;
pub type VkStructureType   = i32;
pub type VkFormat          = i32;
pub type VkColorSpaceKHR   = i32;
pub type VkPresentModeKHR  = i32;
pub type VkImageLayout     = i32;
pub type VkPipelineBindPoint = i32;
pub type VkCommandBufferLevel = i32;
pub type VkImageViewType   = i32;
pub type VkComponentSwizzle= i32;
pub type VkAttachmentLoadOp= i32;
pub type VkAttachmentStoreOp=i32;
pub type VkSampleCountFlagBits = u32;
pub type VkImageUsageFlags = u32;
pub type VkSharingMode     = i32;
pub type VkCompositeAlphaFlagBitsKHR = u32;
pub type VkSurfaceTransformFlagBitsKHR = u32;
pub type VkImageAspectFlags= u32;
pub type VkPipelineStageFlags = u32;
pub type VkAccessFlags     = u32;
pub type VkDependencyFlags = u32;
pub type VkBool32          = u32;
pub type VkDeviceSize      = u64;
pub type VkFlags           = u32;
pub type VkSubpassDescriptionFlags = u32;

// ── VkResult values ───────────────────────────────────────────────────────────
pub const VK_SUCCESS:             VkResult = 0;
pub const VK_SUBOPTIMAL_KHR:      VkResult = 1000001003;
pub const VK_ERROR_OUT_OF_DATE_KHR: VkResult = -1000001004;
pub const VK_NOT_READY:           VkResult = 1;
pub const VK_TIMEOUT:             VkResult = 2;

// ── Structure types ───────────────────────────────────────────────────────────
pub const VK_STRUCTURE_TYPE_APPLICATION_INFO:                    VkStructureType = 0;
pub const VK_STRUCTURE_TYPE_INSTANCE_CREATE_INFO:                VkStructureType = 1;
pub const VK_STRUCTURE_TYPE_DEVICE_QUEUE_CREATE_INFO:            VkStructureType = 2;
pub const VK_STRUCTURE_TYPE_DEVICE_CREATE_INFO:                  VkStructureType = 3;
pub const VK_STRUCTURE_TYPE_SUBMIT_INFO:                         VkStructureType = 4;
pub const VK_STRUCTURE_TYPE_SEMAPHORE_CREATE_INFO:               VkStructureType = 9;
pub const VK_STRUCTURE_TYPE_FENCE_CREATE_INFO:                   VkStructureType = 8;
pub const VK_STRUCTURE_TYPE_IMAGE_VIEW_CREATE_INFO:              VkStructureType = 15;
pub const VK_STRUCTURE_TYPE_RENDER_PASS_CREATE_INFO:             VkStructureType = 38;
pub const VK_STRUCTURE_TYPE_FRAMEBUFFER_CREATE_INFO:             VkStructureType = 37;
pub const VK_STRUCTURE_TYPE_COMMAND_POOL_CREATE_INFO:            VkStructureType = 39;
pub const VK_STRUCTURE_TYPE_COMMAND_BUFFER_ALLOCATE_INFO:        VkStructureType = 40;
pub const VK_STRUCTURE_TYPE_COMMAND_BUFFER_BEGIN_INFO:           VkStructureType = 42;
pub const VK_STRUCTURE_TYPE_RENDER_PASS_BEGIN_INFO:              VkStructureType = 43;
pub const VK_STRUCTURE_TYPE_SWAPCHAIN_CREATE_INFO_KHR:           VkStructureType = 1000001000;
pub const VK_STRUCTURE_TYPE_PRESENT_INFO_KHR:                    VkStructureType = 1000001001;
pub const VK_STRUCTURE_TYPE_WIN32_SURFACE_CREATE_INFO_KHR:       VkStructureType = 1000009000;

// ── Format / colour space ─────────────────────────────────────────────────────
pub const VK_FORMAT_B8G8R8A8_SRGB:       VkFormat = 50;
pub const VK_FORMAT_B8G8R8A8_UNORM:      VkFormat = 44;
pub const VK_COLOR_SPACE_SRGB_NONLINEAR_KHR: VkColorSpaceKHR = 0;

// ── Present modes ─────────────────────────────────────────────────────────────
pub const VK_PRESENT_MODE_IMMEDIATE_KHR: VkPresentModeKHR = 0;
pub const VK_PRESENT_MODE_MAILBOX_KHR:   VkPresentModeKHR = 1;
pub const VK_PRESENT_MODE_FIFO_KHR:      VkPresentModeKHR = 2;

// ── Image flags ───────────────────────────────────────────────────────────────
pub const VK_IMAGE_USAGE_COLOR_ATTACHMENT_BIT: VkImageUsageFlags = 0x00000010;
pub const VK_IMAGE_USAGE_TRANSFER_DST_BIT:     VkImageUsageFlags = 0x00000002;
pub const VK_SHARING_MODE_EXCLUSIVE:           VkSharingMode = 0;
pub const VK_COMPOSITE_ALPHA_OPAQUE_BIT_KHR:   VkCompositeAlphaFlagBitsKHR = 0x00000001;
pub const VK_SURFACE_TRANSFORM_IDENTITY_BIT_KHR: VkSurfaceTransformFlagBitsKHR = 0x00000001;
pub const VK_IMAGE_ASPECT_COLOR_BIT:           VkImageAspectFlags = 0x00000001;
pub const VK_IMAGE_VIEW_TYPE_2D:               VkImageViewType = 1;
pub const VK_COMPONENT_SWIZZLE_IDENTITY:       VkComponentSwizzle = 0;
pub const VK_SAMPLE_COUNT_1_BIT:               VkSampleCountFlagBits = 0x00000001;

// ── Image layouts ─────────────────────────────────────────────────────────────
pub const VK_IMAGE_LAYOUT_UNDEFINED:               VkImageLayout = 0;
pub const VK_IMAGE_LAYOUT_COLOR_ATTACHMENT_OPTIMAL:VkImageLayout = 2;
pub const VK_IMAGE_LAYOUT_PRESENT_SRC_KHR:         VkImageLayout = 1000001002;

// ── Attachment ops ────────────────────────────────────────────────────────────
pub const VK_ATTACHMENT_LOAD_OP_CLEAR:    VkAttachmentLoadOp = 1;
pub const VK_ATTACHMENT_LOAD_OP_DONT_CARE:VkAttachmentLoadOp = 2;
pub const VK_ATTACHMENT_STORE_OP_STORE:   VkAttachmentStoreOp = 0;
pub const VK_ATTACHMENT_STORE_OP_DONT_CARE:VkAttachmentStoreOp = 1;

// ── Misc ──────────────────────────────────────────────────────────────────────
pub const VK_PIPELINE_BIND_POINT_GRAPHICS: VkPipelineBindPoint = 0;
pub const VK_COMMAND_BUFFER_LEVEL_PRIMARY: VkCommandBufferLevel = 0;
pub const VK_PIPELINE_STAGE_COLOR_ATTACHMENT_OUTPUT_BIT: VkPipelineStageFlags = 0x00000400;
pub const VK_FENCE_CREATE_SIGNALED_BIT: VkFlags = 0x00000001;
pub const VK_QUEUE_GRAPHICS_BIT: u32 = 0x00000001;
pub const VK_QUEUE_FAMILY_IGNORED: u32 = u32::MAX;
pub const VK_SUBPASS_EXTERNAL: u32 = u32::MAX;
pub const VK_TRUE:  VkBool32 = 1;
pub const VK_FALSE: VkBool32 = 0;

// ── Pipeline / memory (demo geometry) ─────────────────────────────────────────
pub const VK_FORMAT_R32G32B32_SFLOAT: VkFormat = 106;
pub const VK_FORMAT_D16_UNORM:        VkFormat = 124;
pub const VK_FORMAT_D32_SFLOAT:       VkFormat = 126;
pub const VK_FORMAT_D24_UNORM_S8_UINT: VkFormat = 129;

pub const VK_FORMAT_FEATURE_DEPTH_STENCIL_ATTACHMENT_BIT: u32 = 0x00000200;
pub const VK_IMAGE_TYPE_2D: i32 = 1;
pub const VK_IMAGE_TILING_OPTIMAL: i32 = 0;
pub const VK_IMAGE_TILING_LINEAR: i32 = 1;
pub const VK_IMAGE_USAGE_DEPTH_STENCIL_ATTACHMENT_BIT: VkImageUsageFlags = 0x00000020;
pub const VK_IMAGE_ASPECT_DEPTH_BIT: VkImageAspectFlags = 0x00000002;
pub const VK_IMAGE_LAYOUT_DEPTH_STENCIL_ATTACHMENT_OPTIMAL: VkImageLayout = 3;
pub const VK_BUFFER_USAGE_VERTEX_BUFFER_BIT: VkBufferUsageFlags = 0x00000080;

pub const VK_MEMORY_PROPERTY_DEVICE_LOCAL_BIT:  u32 = 0x00000001;
pub const VK_MEMORY_PROPERTY_HOST_VISIBLE_BIT:  u32 = 0x00000002;
pub const VK_MEMORY_PROPERTY_HOST_COHERENT_BIT: u32 = 0x00000004;

pub const VK_SHADER_STAGE_VERTEX_BIT:   VkShaderStageFlags = 0x00000001;
pub const VK_SHADER_STAGE_FRAGMENT_BIT: VkShaderStageFlags = 0x00000010;
pub const VK_PRIMITIVE_TOPOLOGY_LINE_LIST:      i32 = 1;
pub const VK_PRIMITIVE_TOPOLOGY_TRIANGLE_LIST:  i32 = 3;
pub const VK_POLYGON_MODE_FILL: i32 = 0;
pub const VK_CULL_MODE_BACK_BIT: u32 = 0x00000002;
pub const VK_FRONT_FACE_CLOCKWISE: i32 = 1;
pub const VK_COMPARE_OP_LESS: i32 = 1;
pub const VK_DYNAMIC_STATE_VIEWPORT: i32 = 0;
pub const VK_DYNAMIC_STATE_SCISSOR:  i32 = 1;

pub const VK_ACCESS_COLOR_ATTACHMENT_WRITE_BIT: u32 = 0x00000100;
pub const VK_ACCESS_DEPTH_STENCIL_ATTACHMENT_WRITE_BIT: u32 = 0x00000200;
pub const VK_PIPELINE_STAGE_EARLY_FRAGMENT_TESTS_BIT: u32 = 0x00000100;
pub const VK_PIPELINE_STAGE_LATE_FRAGMENT_TESTS_BIT:  u32 = 0x00000200;

pub const VK_STRUCTURE_TYPE_MEMORY_ALLOCATE_INFO: VkStructureType = 5;
pub const VK_STRUCTURE_TYPE_BUFFER_CREATE_INFO: VkStructureType = 12;
pub const VK_STRUCTURE_TYPE_IMAGE_CREATE_INFO: VkStructureType = 14;
pub const VK_STRUCTURE_TYPE_SHADER_MODULE_CREATE_INFO: VkStructureType = 16;
pub const VK_STRUCTURE_TYPE_PIPELINE_SHADER_STAGE_CREATE_INFO: VkStructureType = 18;
pub const VK_STRUCTURE_TYPE_PIPELINE_VERTEX_INPUT_STATE_CREATE_INFO: VkStructureType = 19;
pub const VK_STRUCTURE_TYPE_PIPELINE_INPUT_ASSEMBLY_STATE_CREATE_INFO: VkStructureType = 20;
pub const VK_STRUCTURE_TYPE_PIPELINE_VIEWPORT_STATE_CREATE_INFO: VkStructureType = 22;
pub const VK_STRUCTURE_TYPE_PIPELINE_RASTERIZATION_STATE_CREATE_INFO: VkStructureType = 23;
pub const VK_STRUCTURE_TYPE_PIPELINE_MULTISAMPLE_STATE_CREATE_INFO: VkStructureType = 24;
pub const VK_STRUCTURE_TYPE_PIPELINE_DEPTH_STENCIL_STATE_CREATE_INFO: VkStructureType = 25;
pub const VK_STRUCTURE_TYPE_PIPELINE_COLOR_BLEND_STATE_CREATE_INFO: VkStructureType = 26;
pub const VK_STRUCTURE_TYPE_PIPELINE_DYNAMIC_STATE_CREATE_INFO: VkStructureType = 27;
pub const VK_STRUCTURE_TYPE_GRAPHICS_PIPELINE_CREATE_INFO: VkStructureType = 28;
pub const VK_STRUCTURE_TYPE_PIPELINE_LAYOUT_CREATE_INFO: VkStructureType = 30;

pub type VkBufferUsageFlags = u32;
pub type VkShaderStageFlags = u32;

// ── Structs ───────────────────────────────────────────────────────────────────

#[repr(C)]
pub struct VkApplicationInfo {
    pub sType:              VkStructureType,
    pub pNext:              *const c_void,
    pub pApplicationName:   *const i8,
    pub applicationVersion: u32,
    pub pEngineName:        *const i8,
    pub engineVersion:      u32,
    pub apiVersion:         u32,
}

#[repr(C)]
pub struct VkInstanceCreateInfo {
    pub sType:                   VkStructureType,
    pub pNext:                   *const c_void,
    pub flags:                   VkFlags,
    pub pApplicationInfo:        *const VkApplicationInfo,
    pub enabledLayerCount:       u32,
    pub ppEnabledLayerNames:     *const *const i8,
    pub enabledExtensionCount:   u32,
    pub ppEnabledExtensionNames: *const *const i8,
}

#[repr(C)]
pub struct VkDeviceQueueCreateInfo {
    pub sType:            VkStructureType,
    pub pNext:            *const c_void,
    pub flags:            VkFlags,
    pub queueFamilyIndex: u32,
    pub queueCount:       u32,
    pub pQueuePriorities: *const f32,
}

#[repr(C)]
pub struct VkPhysicalDeviceFeatures {
    pub fields: [VkBool32; 55], // sparse boolean feature flags
}
impl VkPhysicalDeviceFeatures {
    pub fn zeroed() -> Self { unsafe { std::mem::zeroed() } }
}

#[repr(C)]
pub struct VkDeviceCreateInfo {
    pub sType:                   VkStructureType,
    pub pNext:                   *const c_void,
    pub flags:                   VkFlags,
    pub queueCreateInfoCount:    u32,
    pub pQueueCreateInfos:       *const VkDeviceQueueCreateInfo,
    pub enabledLayerCount:       u32,
    pub ppEnabledLayerNames:     *const *const i8,
    pub enabledExtensionCount:   u32,
    pub ppEnabledExtensionNames: *const *const i8,
    pub pEnabledFeatures:        *const VkPhysicalDeviceFeatures,
}

#[repr(C)]
pub struct VkWin32SurfaceCreateInfoKHR {
    pub sType:     VkStructureType,
    pub pNext:     *const c_void,
    pub flags:     VkFlags,
    pub hinstance: *mut c_void,
    pub hwnd:      *mut c_void,
}

#[repr(C)] #[derive(Clone, Copy)]
pub struct VkSurfaceFormatKHR {
    pub format:     VkFormat,
    pub colorSpace: VkColorSpaceKHR,
}

#[repr(C)] #[derive(Clone, Copy, Default)]
pub struct VkExtent2D { pub width: u32, pub height: u32 }

#[repr(C)] #[derive(Clone, Copy)]
pub struct VkSurfaceCapabilitiesKHR {
    pub minImageCount:           u32,
    pub maxImageCount:           u32,
    pub currentExtent:           VkExtent2D,
    pub minImageExtent:          VkExtent2D,
    pub maxImageExtent:          VkExtent2D,
    pub maxImageArrayLayers:     u32,
    pub supportedTransforms:     u32,
    pub currentTransform:        VkSurfaceTransformFlagBitsKHR,
    pub supportedCompositeAlpha: u32,
    pub supportedUsageFlags:     VkImageUsageFlags,
}
impl VkSurfaceCapabilitiesKHR {
    pub fn zeroed() -> Self { unsafe { std::mem::zeroed() } }
}

#[repr(C)]
pub struct VkSwapchainCreateInfoKHR {
    pub sType:                 VkStructureType,
    pub pNext:                 *const c_void,
    pub flags:                 VkFlags,
    pub surface:               VkSurfaceKHR,
    pub minImageCount:         u32,
    pub imageFormat:           VkFormat,
    pub imageColorSpace:       VkColorSpaceKHR,
    pub imageExtent:           VkExtent2D,
    pub imageArrayLayers:      u32,
    pub imageUsage:            VkImageUsageFlags,
    pub imageSharingMode:      VkSharingMode,
    pub queueFamilyIndexCount: u32,
    pub pQueueFamilyIndices:   *const u32,
    pub preTransform:          VkSurfaceTransformFlagBitsKHR,
    pub compositeAlpha:        VkCompositeAlphaFlagBitsKHR,
    pub presentMode:           VkPresentModeKHR,
    pub clipped:               VkBool32,
    pub oldSwapchain:          VkSwapchainKHR,
}

#[repr(C)]
pub struct VkComponentMapping {
    pub r: VkComponentSwizzle,
    pub g: VkComponentSwizzle,
    pub b: VkComponentSwizzle,
    pub a: VkComponentSwizzle,
}

#[repr(C)]
pub struct VkImageSubresourceRange {
    pub aspectMask:     VkImageAspectFlags,
    pub baseMipLevel:   u32,
    pub levelCount:     u32,
    pub baseArrayLayer: u32,
    pub layerCount:     u32,
}

#[repr(C)]
pub struct VkImageViewCreateInfo {
    pub sType:            VkStructureType,
    pub pNext:            *const c_void,
    pub flags:            VkFlags,
    pub image:            VkImage,
    pub viewType:         VkImageViewType,
    pub format:           VkFormat,
    pub components:       VkComponentMapping,
    pub subresourceRange: VkImageSubresourceRange,
}

#[repr(C)]
pub struct VkAttachmentDescription {
    pub flags:          VkFlags,
    pub format:         VkFormat,
    pub samples:        VkSampleCountFlagBits,
    pub loadOp:         VkAttachmentLoadOp,
    pub storeOp:        VkAttachmentStoreOp,
    pub stencilLoadOp:  VkAttachmentLoadOp,
    pub stencilStoreOp: VkAttachmentStoreOp,
    pub initialLayout:  VkImageLayout,
    pub finalLayout:    VkImageLayout,
}

#[repr(C)]
pub struct VkAttachmentReference {
    pub attachment: u32,
    pub layout:     VkImageLayout,
}

#[repr(C)]
pub struct VkSubpassDescription {
    pub flags:                   VkSubpassDescriptionFlags,
    pub pipelineBindPoint:       VkPipelineBindPoint,
    pub inputAttachmentCount:    u32,
    pub pInputAttachments:       *const VkAttachmentReference,
    pub colorAttachmentCount:    u32,
    pub pColorAttachments:       *const VkAttachmentReference,
    pub pResolveAttachments:     *const VkAttachmentReference,
    pub pDepthStencilAttachment: *const VkAttachmentReference,
    pub preserveAttachmentCount: u32,
    pub pPreserveAttachments:    *const u32,
}

#[repr(C)]
pub struct VkSubpassDependency {
    pub srcSubpass:      u32,
    pub dstSubpass:      u32,
    pub srcStageMask:    VkPipelineStageFlags,
    pub dstStageMask:    VkPipelineStageFlags,
    pub srcAccessMask:   VkAccessFlags,
    pub dstAccessMask:   VkAccessFlags,
    pub dependencyFlags: VkDependencyFlags,
}

#[repr(C)]
pub struct VkRenderPassCreateInfo {
    pub sType:           VkStructureType,
    pub pNext:           *const c_void,
    pub flags:           VkFlags,
    pub attachmentCount: u32,
    pub pAttachments:    *const VkAttachmentDescription,
    pub subpassCount:    u32,
    pub pSubpasses:      *const VkSubpassDescription,
    pub dependencyCount: u32,
    pub pDependencies:   *const VkSubpassDependency,
}

#[repr(C)]
pub struct VkFramebufferCreateInfo {
    pub sType:           VkStructureType,
    pub pNext:           *const c_void,
    pub flags:           VkFlags,
    pub renderPass:      VkRenderPass,
    pub attachmentCount: u32,
    pub pAttachments:    *const VkImageView,
    pub width:           u32,
    pub height:          u32,
    pub layers:          u32,
}

#[repr(C)]
pub struct VkCommandPoolCreateInfo {
    pub sType:            VkStructureType,
    pub pNext:            *const c_void,
    pub flags:            VkFlags,
    pub queueFamilyIndex: u32,
}

#[repr(C)]
pub struct VkCommandBufferAllocateInfo {
    pub sType:              VkStructureType,
    pub pNext:              *const c_void,
    pub commandPool:        VkCommandPool,
    pub level:              VkCommandBufferLevel,
    pub commandBufferCount: u32,
}

#[repr(C)]
pub struct VkCommandBufferBeginInfo {
    pub sType:            VkStructureType,
    pub pNext:            *const c_void,
    pub flags:            VkFlags,
    pub pInheritanceInfo: *const c_void,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct VkClearColorValue { pub float32: [f32; 4] }

#[repr(C)]
pub union VkClearValue {
    pub color:        VkClearColorValue,
    pub depthStencil: VkClearDepthStencilValue,
}

#[repr(C)] #[derive(Clone, Copy)]
pub struct VkClearDepthStencilValue { pub depth: f32, pub stencil: u32 }

#[repr(C)]
pub struct VkOffset2D { pub x: i32, pub y: i32 }

#[repr(C)]
pub struct VkRect2D { pub offset: VkOffset2D, pub extent: VkExtent2D }

#[repr(C)]
pub struct VkRenderPassBeginInfo {
    pub sType:           VkStructureType,
    pub pNext:           *const c_void,
    pub renderPass:      VkRenderPass,
    pub framebuffer:     VkFramebuffer,
    pub renderArea:      VkRect2D,
    pub clearValueCount: u32,
    pub pClearValues:    *const VkClearValue,
}

#[repr(C)]
pub struct VkSemaphoreCreateInfo {
    pub sType: VkStructureType,
    pub pNext: *const c_void,
    pub flags: VkFlags,
}

#[repr(C)]
pub struct VkFenceCreateInfo {
    pub sType: VkStructureType,
    pub pNext: *const c_void,
    pub flags: VkFlags,
}

#[repr(C)]
pub struct VkSubmitInfo {
    pub sType:                VkStructureType,
    pub pNext:                *const c_void,
    pub waitSemaphoreCount:   u32,
    pub pWaitSemaphores:      *const VkSemaphore,
    pub pWaitDstStageMask:    *const VkPipelineStageFlags,
    pub commandBufferCount:   u32,
    pub pCommandBuffers:      *const VkCommandBuffer,
    pub signalSemaphoreCount: u32,
    pub pSignalSemaphores:    *const VkSemaphore,
}

#[repr(C)]
pub struct VkPresentInfoKHR {
    pub sType:              VkStructureType,
    pub pNext:              *const c_void,
    pub waitSemaphoreCount: u32,
    pub pWaitSemaphores:    *const VkSemaphore,
    pub swapchainCount:     u32,
    pub pSwapchains:        *const VkSwapchainKHR,
    pub pImageIndices:      *const u32,
    pub pResults:           *mut VkResult,
}

#[repr(C)] #[derive(Clone, Copy)]
pub struct VkQueueFamilyProperties {
    pub queueFlags:                  u32,
    pub queueCount:                  u32,
    pub timestampValidBits:          u32,
    pub minImageTransferGranularity: VkExtent3D,
}

#[repr(C)] #[derive(Clone, Copy)]
pub struct VkExtent3D { pub width: u32, pub height: u32, pub depth: u32 }

#[repr(C)] #[derive(Clone, Copy)]
pub struct VkPhysicalDeviceLimits {
    pub _data: [u8; 504],
}

#[repr(C)] #[derive(Clone, Copy)]
pub struct VkPhysicalDeviceSparseProperties {
    pub residencyStandard2DBlockShape: VkBool32,
    pub residencyStandard2DMultisampleBlockShape: VkBool32,
    pub residencyStandard3DBlockShape: VkBool32,
    pub residencyAlignedMipSize: VkBool32,
    pub residencyNonResidentStrict: VkBool32,
}

#[repr(C)] #[derive(Clone, Copy)]
pub struct VkPhysicalDeviceProperties {
    pub apiVersion: u32,
    pub driverVersion: u32,
    pub vendorID: u32,
    pub deviceID: u32,
    pub deviceType: u32,
    pub deviceName: [i8; 256],
    pub pipelineCacheUUID: [u8; 16],
    pub limits: VkPhysicalDeviceLimits,
    pub sparseProperties: VkPhysicalDeviceSparseProperties,
}
impl VkPhysicalDeviceProperties {
    pub fn zeroed() -> Self { unsafe { std::mem::zeroed() } }
}


#[repr(C)]
pub struct VkFormatProperties {
    pub linearTilingFeatures:  u32,
    pub optimalTilingFeatures: u32,
    pub bufferFeatures:        u32,
}

#[repr(C)] #[derive(Clone, Copy)]
pub struct VkMemoryType {
    pub propertyFlags: u32,
    pub heapIndex:     u32,
}

#[repr(C)] #[derive(Clone, Copy)]
pub struct VkMemoryHeap {
    pub size:  VkDeviceSize,
    pub flags: VkFlags,
}

#[repr(C)] #[derive(Clone, Copy)]
pub struct VkPhysicalDeviceMemoryProperties {
    pub memoryTypeCount: u32,
    pub memoryTypes:     [VkMemoryType; 32],
    pub memoryHeapCount: u32,
    pub memoryHeaps:     [VkMemoryHeap; 16],
}

#[repr(C)]
pub struct VkMemoryRequirements {
    pub size:            VkDeviceSize,
    pub alignment:       VkDeviceSize,
    pub memoryTypeBits:  u32,
}

#[repr(C)]
pub struct VkMemoryAllocateInfo {
    pub sType:           VkStructureType,
    pub pNext:           *const c_void,
    pub allocationSize:  VkDeviceSize,
    pub memoryTypeIndex: u32,
}

#[repr(C)]
pub struct VkBufferCreateInfo {
    pub sType:                 VkStructureType,
    pub pNext:                 *const c_void,
    pub flags:                 VkFlags,
    pub size:                  VkDeviceSize,
    pub usage:                 VkBufferUsageFlags,
    pub sharingMode:           VkSharingMode,
    pub queueFamilyIndexCount: u32,
    pub pQueueFamilyIndices:   *const u32,
}

#[repr(C)]
pub struct VkImageCreateInfo {
    pub sType:                 VkStructureType,
    pub pNext:                 *const c_void,
    pub flags:                 VkFlags,
    pub imageType:             i32,
    pub format:                VkFormat,
    pub extent:                VkExtent3D,
    pub mipLevels:             u32,
    pub arrayLayers:           u32,
    pub samples:               VkSampleCountFlagBits,
    pub tiling:                i32,
    pub usage:                 VkImageUsageFlags,
    pub sharingMode:           VkSharingMode,
    pub queueFamilyIndexCount: u32,
    pub pQueueFamilyIndices:   *const u32,
    pub initialLayout:         VkImageLayout,
}

#[repr(C)]
pub struct VkShaderModuleCreateInfo {
    pub sType:    VkStructureType,
    pub pNext:    *const c_void,
    pub flags:    VkFlags,
    pub codeSize: usize,
    pub pCode:    *const u32,
}

#[repr(C)]
pub struct VkPushConstantRange {
    pub stageFlags: VkShaderStageFlags,
    pub offset:     u32,
    pub size:       u32,
}

#[repr(C)]
pub struct VkPipelineLayoutCreateInfo {
    pub sType:                  VkStructureType,
    pub pNext:                  *const c_void,
    pub flags:                  VkFlags,
    pub setLayoutCount:         u32,
    pub pSetLayouts:            *const VkDescriptorSetLayout,
    pub pushConstantRangeCount: u32,
    pub pPushConstantRanges:    *const VkPushConstantRange,
}

#[repr(C)]
pub struct VkPipelineShaderStageCreateInfo {
    pub sType:               VkStructureType,
    pub pNext:               *const c_void,
    pub flags:               VkFlags,
    pub stage:               VkShaderStageFlags,
    pub module:              VkShaderModule,
    pub pName:               *const i8,
    pub pSpecializationInfo: *const c_void,
}

#[repr(C)] #[derive(Clone, Copy)]
pub struct VkVertexInputBindingDescription {
    pub binding:   u32,
    pub stride:    u32,
    pub inputRate: u32,
}

#[repr(C)] #[derive(Clone, Copy)]
pub struct VkVertexInputAttributeDescription {
    pub location: u32,
    pub binding:  u32,
    pub format:   VkFormat,
    pub offset:   u32,
}

#[repr(C)]
pub struct VkPipelineVertexInputStateCreateInfo {
    pub sType:                           VkStructureType,
    pub pNext:                           *const c_void,
    pub flags:                           VkFlags,
    pub vertexBindingDescriptionCount:   u32,
    pub pVertexBindingDescriptions:      *const VkVertexInputBindingDescription,
    pub vertexAttributeDescriptionCount: u32,
    pub pVertexAttributeDescriptions:    *const VkVertexInputAttributeDescription,
}

#[repr(C)]
pub struct VkPipelineInputAssemblyStateCreateInfo {
    pub sType:                  VkStructureType,
    pub pNext:                  *const c_void,
    pub flags:                  VkFlags,
    pub topology:               i32,
    pub primitiveRestartEnable: VkBool32,
}

#[repr(C)]
pub struct VkViewport {
    pub x:        f32,
    pub y:        f32,
    pub width:    f32,
    pub height:   f32,
    pub minDepth: f32,
    pub maxDepth: f32,
}

#[repr(C)]
pub struct VkPipelineViewportStateCreateInfo {
    pub sType:         VkStructureType,
    pub pNext:         *const c_void,
    pub flags:         VkFlags,
    pub viewportCount: u32,
    pub pViewports:    *const VkViewport,
    pub scissorCount:  u32,
    pub pScissors:     *const VkRect2D,
}

#[repr(C)]
pub struct VkPipelineRasterizationStateCreateInfo {
    pub sType:                   VkStructureType,
    pub pNext:                   *const c_void,
    pub flags:                   VkFlags,
    pub depthClampEnable:        VkBool32,
    pub rasterizerDiscardEnable: VkBool32,
    pub polygonMode:             i32,
    pub cullMode:                u32,
    pub frontFace:               i32,
    pub depthBiasEnable:         VkBool32,
    pub depthBiasConstantFactor: f32,
    pub depthBiasClamp:          f32,
    pub depthBiasSlopeFactor:    f32,
    pub lineWidth:               f32,
}

#[repr(C)]
pub struct VkPipelineMultisampleStateCreateInfo {
    pub sType:                 VkStructureType,
    pub pNext:                 *const c_void,
    pub flags:                 VkFlags,
    pub rasterizationSamples:  VkSampleCountFlagBits,
    pub sampleShadingEnable:   VkBool32,
    pub minSampleShading:      f32,
    pub pSampleMask:           *const u32,
    pub alphaToCoverageEnable: VkBool32,
    pub alphaToOneEnable:      VkBool32,
}

#[repr(C)] #[derive(Clone, Copy)]
pub struct VkStencilOpState {
    pub failOp:      i32,
    pub passOp:      i32,
    pub depthFailOp: i32,
    pub compareOp:   i32,
    pub compareMask: u32,
    pub writeMask:   u32,
    pub reference:   u32,
}

#[repr(C)]
pub struct VkPipelineDepthStencilStateCreateInfo {
    pub sType:                 VkStructureType,
    pub pNext:                 *const c_void,
    pub flags:                 VkFlags,
    pub depthTestEnable:       VkBool32,
    pub depthWriteEnable:      VkBool32,
    pub depthCompareOp:        i32,
    pub depthBoundsTestEnable: VkBool32,
    pub stencilTestEnable:     VkBool32,
    pub front:                 VkStencilOpState,
    pub back:                  VkStencilOpState,
    pub minDepthBounds:        f32,
    pub maxDepthBounds:        f32,
}

#[repr(C)]
pub struct VkPipelineColorBlendAttachmentState {
    pub blendEnable:         VkBool32,
    pub srcColorBlendFactor: i32,
    pub dstColorBlendFactor: i32,
    pub colorBlendOp:        i32,
    pub srcAlphaBlendFactor: i32,
    pub dstAlphaBlendFactor: i32,
    pub alphaBlendOp:        i32,
    pub colorWriteMask:      u32,
}

#[repr(C)]
pub struct VkPipelineColorBlendStateCreateInfo {
    pub sType:           VkStructureType,
    pub pNext:           *const c_void,
    pub flags:           VkFlags,
    pub logicOpEnable:   VkBool32,
    pub logicOp:         i32,
    pub attachmentCount: u32,
    pub pAttachments:    *const VkPipelineColorBlendAttachmentState,
    pub blendConstants:  [f32; 4],
}

#[repr(C)]
pub struct VkPipelineDynamicStateCreateInfo {
    pub sType:             VkStructureType,
    pub pNext:             *const c_void,
    pub flags:             VkFlags,
    pub dynamicStateCount: u32,
    pub pDynamicStates:    *const i32,
}

#[repr(C)]
pub struct VkGraphicsPipelineCreateInfo {
    pub sType:               VkStructureType,
    pub pNext:               *const c_void,
    pub flags:               VkFlags,
    pub stageCount:          u32,
    pub pStages:             *const VkPipelineShaderStageCreateInfo,
    pub pVertexInputState:   *const VkPipelineVertexInputStateCreateInfo,
    pub pInputAssemblyState: *const VkPipelineInputAssemblyStateCreateInfo,
    pub pTessellationState:  *const c_void,
    pub pViewportState:      *const VkPipelineViewportStateCreateInfo,
    pub pRasterizationState: *const VkPipelineRasterizationStateCreateInfo,
    pub pMultisampleState:   *const VkPipelineMultisampleStateCreateInfo,
    pub pDepthStencilState:  *const VkPipelineDepthStencilStateCreateInfo,
    pub pColorBlendState:    *const VkPipelineColorBlendStateCreateInfo,
    pub pDynamicState:       *const VkPipelineDynamicStateCreateInfo,
    pub layout:              VkPipelineLayout,
    pub renderPass:          VkRenderPass,
    pub subpass:             u32,
    pub basePipelineHandle:  VkPipeline,
    pub basePipelineIndex:   i32,
}

// ── Function pointer types ────────────────────────────────────────────────────

pub type PFN_vkGetInstanceProcAddr =
    unsafe extern "system" fn(instance: VkInstance, pName: *const i8) -> *const c_void;
pub type PFN_vkCreateInstance =
    unsafe extern "system" fn(pCreateInfo: *const VkInstanceCreateInfo, pAllocator: *const c_void, pInstance: *mut VkInstance) -> VkResult;
pub type PFN_vkDestroyInstance =
    unsafe extern "system" fn(instance: VkInstance, pAllocator: *const c_void);
pub type PFN_vkEnumeratePhysicalDevices =
    unsafe extern "system" fn(instance: VkInstance, pPhysicalDeviceCount: *mut u32, pPhysicalDevices: *mut VkPhysicalDevice) -> VkResult;
pub type PFN_vkGetPhysicalDeviceProperties =
    unsafe extern "system" fn(physicalDevice: VkPhysicalDevice, pProperties: *mut VkPhysicalDeviceProperties);
pub type PFN_vkGetPhysicalDeviceQueueFamilyProperties =
    unsafe extern "system" fn(physicalDevice: VkPhysicalDevice, pQueueFamilyPropertyCount: *mut u32, pQueueFamilyProperties: *mut VkQueueFamilyProperties);
pub type PFN_vkCreateDevice =
    unsafe extern "system" fn(physicalDevice: VkPhysicalDevice, pCreateInfo: *const VkDeviceCreateInfo, pAllocator: *const c_void, pDevice: *mut VkDevice) -> VkResult;
pub type PFN_vkDestroyDevice =
    unsafe extern "system" fn(device: VkDevice, pAllocator: *const c_void);
pub type PFN_vkGetDeviceQueue =
    unsafe extern "system" fn(device: VkDevice, queueFamilyIndex: u32, queueIndex: u32, pQueue: *mut VkQueue);
pub type PFN_vkDeviceWaitIdle =
    unsafe extern "system" fn(device: VkDevice) -> VkResult;
pub type PFN_vkCreateWin32SurfaceKHR =
    unsafe extern "system" fn(instance: VkInstance, pCreateInfo: *const VkWin32SurfaceCreateInfoKHR, pAllocator: *const c_void, pSurface: *mut VkSurfaceKHR) -> VkResult;
pub type PFN_vkDestroySurfaceKHR =
    unsafe extern "system" fn(instance: VkInstance, surface: VkSurfaceKHR, pAllocator: *const c_void);
pub type PFN_vkGetPhysicalDeviceSurfaceSupportKHR =
    unsafe extern "system" fn(physicalDevice: VkPhysicalDevice, queueFamilyIndex: u32, surface: VkSurfaceKHR, pSupported: *mut VkBool32) -> VkResult;
pub type PFN_vkGetPhysicalDeviceSurfaceCapabilitiesKHR =
    unsafe extern "system" fn(physicalDevice: VkPhysicalDevice, surface: VkSurfaceKHR, pSurfaceCapabilities: *mut VkSurfaceCapabilitiesKHR) -> VkResult;
pub type PFN_vkGetPhysicalDeviceSurfaceFormatsKHR =
    unsafe extern "system" fn(physicalDevice: VkPhysicalDevice, surface: VkSurfaceKHR, pSurfaceFormatCount: *mut u32, pSurfaceFormats: *mut VkSurfaceFormatKHR) -> VkResult;
pub type PFN_vkGetPhysicalDeviceSurfacePresentModesKHR =
    unsafe extern "system" fn(physicalDevice: VkPhysicalDevice, surface: VkSurfaceKHR, pPresentModeCount: *mut u32, pPresentModes: *mut VkPresentModeKHR) -> VkResult;
pub type PFN_vkCreateSwapchainKHR =
    unsafe extern "system" fn(device: VkDevice, pCreateInfo: *const VkSwapchainCreateInfoKHR, pAllocator: *const c_void, pSwapchain: *mut VkSwapchainKHR) -> VkResult;
pub type PFN_vkDestroySwapchainKHR =
    unsafe extern "system" fn(device: VkDevice, swapchain: VkSwapchainKHR, pAllocator: *const c_void);
pub type PFN_vkGetSwapchainImagesKHR =
    unsafe extern "system" fn(device: VkDevice, swapchain: VkSwapchainKHR, pSwapchainImageCount: *mut u32, pSwapchainImages: *mut VkImage) -> VkResult;
pub type PFN_vkCreateImageView =
    unsafe extern "system" fn(device: VkDevice, pCreateInfo: *const VkImageViewCreateInfo, pAllocator: *const c_void, pView: *mut VkImageView) -> VkResult;
pub type PFN_vkDestroyImageView =
    unsafe extern "system" fn(device: VkDevice, imageView: VkImageView, pAllocator: *const c_void);
pub type PFN_vkCreateRenderPass =
    unsafe extern "system" fn(device: VkDevice, pCreateInfo: *const VkRenderPassCreateInfo, pAllocator: *const c_void, pRenderPass: *mut VkRenderPass) -> VkResult;
pub type PFN_vkDestroyRenderPass =
    unsafe extern "system" fn(device: VkDevice, renderPass: VkRenderPass, pAllocator: *const c_void);
pub type PFN_vkCreateFramebuffer =
    unsafe extern "system" fn(device: VkDevice, pCreateInfo: *const VkFramebufferCreateInfo, pAllocator: *const c_void, pFramebuffer: *mut VkFramebuffer) -> VkResult;
pub type PFN_vkDestroyFramebuffer =
    unsafe extern "system" fn(device: VkDevice, framebuffer: VkFramebuffer, pAllocator: *const c_void);
pub type PFN_vkCreateCommandPool =
    unsafe extern "system" fn(device: VkDevice, pCreateInfo: *const VkCommandPoolCreateInfo, pAllocator: *const c_void, pCommandPool: *mut VkCommandPool) -> VkResult;
pub type PFN_vkDestroyCommandPool =
    unsafe extern "system" fn(device: VkDevice, commandPool: VkCommandPool, pAllocator: *const c_void);
pub type PFN_vkAllocateCommandBuffers =
    unsafe extern "system" fn(device: VkDevice, pAllocateInfo: *const VkCommandBufferAllocateInfo, pCommandBuffers: *mut VkCommandBuffer) -> VkResult;
pub type PFN_vkBeginCommandBuffer =
    unsafe extern "system" fn(commandBuffer: VkCommandBuffer, pBeginInfo: *const VkCommandBufferBeginInfo) -> VkResult;
pub type PFN_vkEndCommandBuffer =
    unsafe extern "system" fn(commandBuffer: VkCommandBuffer) -> VkResult;
pub type PFN_vkCmdBeginRenderPass =
    unsafe extern "system" fn(commandBuffer: VkCommandBuffer, pRenderPassBegin: *const VkRenderPassBeginInfo, contents: i32);
pub type PFN_vkCmdEndRenderPass =
    unsafe extern "system" fn(commandBuffer: VkCommandBuffer);
pub type PFN_vkCreateSemaphore =
    unsafe extern "system" fn(device: VkDevice, pCreateInfo: *const VkSemaphoreCreateInfo, pAllocator: *const c_void, pSemaphore: *mut VkSemaphore) -> VkResult;
pub type PFN_vkDestroySemaphore =
    unsafe extern "system" fn(device: VkDevice, semaphore: VkSemaphore, pAllocator: *const c_void);
pub type PFN_vkCreateFence =
    unsafe extern "system" fn(device: VkDevice, pCreateInfo: *const VkFenceCreateInfo, pAllocator: *const c_void, pFence: *mut VkFence) -> VkResult;
pub type PFN_vkDestroyFence =
    unsafe extern "system" fn(device: VkDevice, fence: VkFence, pAllocator: *const c_void);
pub type PFN_vkWaitForFences =
    unsafe extern "system" fn(device: VkDevice, fenceCount: u32, pFences: *const VkFence, waitAll: VkBool32, timeout: u64) -> VkResult;
pub type PFN_vkResetFences =
    unsafe extern "system" fn(device: VkDevice, fenceCount: u32, pFences: *const VkFence) -> VkResult;
pub type PFN_vkAcquireNextImageKHR =
    unsafe extern "system" fn(device: VkDevice, swapchain: VkSwapchainKHR, timeout: u64, semaphore: VkSemaphore, fence: VkFence, pImageIndex: *mut u32) -> VkResult;
pub type PFN_vkQueueSubmit =
    unsafe extern "system" fn(queue: VkQueue, submitCount: u32, pSubmits: *const VkSubmitInfo, fence: VkFence) -> VkResult;
pub type PFN_vkQueuePresentKHR =
    unsafe extern "system" fn(queue: VkQueue, pPresentInfo: *const VkPresentInfoKHR) -> VkResult;
pub type PFN_vkResetCommandBuffer =
    unsafe extern "system" fn(commandBuffer: VkCommandBuffer, flags: VkFlags) -> VkResult;
pub type PFN_vkGetPhysicalDeviceMemoryProperties =
    unsafe extern "system" fn(physicalDevice: VkPhysicalDevice, pMemoryProperties: *mut VkPhysicalDeviceMemoryProperties);
pub type PFN_vkGetPhysicalDeviceFormatProperties =
    unsafe extern "system" fn(physicalDevice: VkPhysicalDevice, format: VkFormat, pFormatProperties: *mut VkFormatProperties);
pub type PFN_vkCreateShaderModule =
    unsafe extern "system" fn(device: VkDevice, pCreateInfo: *const VkShaderModuleCreateInfo, pAllocator: *const c_void, pShaderModule: *mut VkShaderModule) -> VkResult;
pub type PFN_vkDestroyShaderModule =
    unsafe extern "system" fn(device: VkDevice, shaderModule: VkShaderModule, pAllocator: *const c_void);
pub type PFN_vkCreatePipelineLayout =
    unsafe extern "system" fn(device: VkDevice, pCreateInfo: *const VkPipelineLayoutCreateInfo, pAllocator: *const c_void, pPipelineLayout: *mut VkPipelineLayout) -> VkResult;
pub type PFN_vkDestroyPipelineLayout =
    unsafe extern "system" fn(device: VkDevice, pipelineLayout: VkPipelineLayout, pAllocator: *const c_void);
pub type PFN_vkCreateGraphicsPipelines =
    unsafe extern "system" fn(device: VkDevice, pipelineCache: u64, createInfoCount: u32, pCreateInfos: *const VkGraphicsPipelineCreateInfo, pAllocator: *const c_void, pPipelines: *mut VkPipeline) -> VkResult;
pub type PFN_vkDestroyPipeline =
    unsafe extern "system" fn(device: VkDevice, pipeline: VkPipeline, pAllocator: *const c_void);
pub type PFN_vkCreateBuffer =
    unsafe extern "system" fn(device: VkDevice, pCreateInfo: *const VkBufferCreateInfo, pAllocator: *const c_void, pBuffer: *mut VkBuffer) -> VkResult;
pub type PFN_vkDestroyBuffer =
    unsafe extern "system" fn(device: VkDevice, buffer: VkBuffer, pAllocator: *const c_void);
pub type PFN_vkGetBufferMemoryRequirements =
    unsafe extern "system" fn(device: VkDevice, buffer: VkBuffer, pMemoryRequirements: *mut VkMemoryRequirements);
pub type PFN_vkAllocateMemory =
    unsafe extern "system" fn(device: VkDevice, pAllocateInfo: *const VkMemoryAllocateInfo, pAllocator: *const c_void, pMemory: *mut VkDeviceMemory) -> VkResult;
pub type PFN_vkFreeMemory =
    unsafe extern "system" fn(device: VkDevice, memory: VkDeviceMemory, pAllocator: *const c_void);
pub type PFN_vkBindBufferMemory =
    unsafe extern "system" fn(device: VkDevice, buffer: VkBuffer, memory: VkDeviceMemory, memoryOffset: VkDeviceSize) -> VkResult;
pub type PFN_vkMapMemory =
    unsafe extern "system" fn(device: VkDevice, memory: VkDeviceMemory, offset: VkDeviceSize, size: VkDeviceSize, flags: VkFlags, ppData: *mut *mut c_void) -> VkResult;
pub type PFN_vkUnmapMemory =
    unsafe extern "system" fn(device: VkDevice, memory: VkDeviceMemory);
pub type PFN_vkCreateImage =
    unsafe extern "system" fn(device: VkDevice, pCreateInfo: *const VkImageCreateInfo, pAllocator: *const c_void, pImage: *mut VkImage) -> VkResult;
pub type PFN_vkDestroyImage =
    unsafe extern "system" fn(device: VkDevice, image: VkImage, pAllocator: *const c_void);
pub type PFN_vkGetImageMemoryRequirements =
    unsafe extern "system" fn(device: VkDevice, image: VkImage, pMemoryRequirements: *mut VkMemoryRequirements);
pub type PFN_vkBindImageMemory =
    unsafe extern "system" fn(device: VkDevice, image: VkImage, memory: VkDeviceMemory, memoryOffset: VkDeviceSize) -> VkResult;
pub type PFN_vkCmdBindPipeline =
    unsafe extern "system" fn(commandBuffer: VkCommandBuffer, pipelineBindPoint: VkPipelineBindPoint, pipeline: VkPipeline);
pub type PFN_vkCmdSetViewport =
    unsafe extern "system" fn(commandBuffer: VkCommandBuffer, firstViewport: u32, viewportCount: u32, pViewports: *const VkViewport);
pub type PFN_vkCmdSetScissor =
    unsafe extern "system" fn(commandBuffer: VkCommandBuffer, firstScissor: u32, scissorCount: u32, pScissors: *const VkRect2D);
pub type PFN_vkCmdBindVertexBuffers =
    unsafe extern "system" fn(commandBuffer: VkCommandBuffer, firstBinding: u32, bindingCount: u32, pBuffers: *const VkBuffer, pOffsets: *const VkDeviceSize);
pub type PFN_vkCmdPushConstants =
    unsafe extern "system" fn(commandBuffer: VkCommandBuffer, layout: VkPipelineLayout, stageFlags: VkShaderStageFlags, offset: u32, size: u32, pValues: *const c_void);
pub type PFN_vkCmdDraw =
    unsafe extern "system" fn(commandBuffer: VkCommandBuffer, vertexCount: u32, instanceCount: u32, firstVertex: u32, firstInstance: u32);

// ── API version helper ────────────────────────────────────────────────────────
pub const fn vk_make_version(major: u32, minor: u32, patch: u32) -> u32 {
    (major << 22) | (minor << 12) | patch
}
pub const VK_API_VERSION_1_0: u32 = vk_make_version(1, 0, 0);
pub const VK_API_VERSION_1_2: u32 = vk_make_version(1, 2, 0);
pub const VK_API_VERSION_1_3: u32 = vk_make_version(1, 3, 0);

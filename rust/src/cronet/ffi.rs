#![allow(non_camel_case_types, dead_code)]
use std::os::raw::{c_char, c_int, c_void};

pub type Cronet_EnginePtr = *mut c_void;
pub type Cronet_EngineParamsPtr = *mut c_void;
pub type Cronet_UrlRequestPtr = *mut c_void;
pub type Cronet_UrlRequestParamsPtr = *mut c_void;
pub type Cronet_UrlRequestCallbackPtr = *mut c_void;
pub type Cronet_UrlResponseInfoPtr = *mut c_void;
pub type Cronet_ExecutorPtr = *mut c_void;
pub type Cronet_RunnablePtr = *mut c_void;
pub type Cronet_BufferPtr = *mut c_void;
pub type Cronet_HttpHeaderPtr = *mut c_void;
pub type Cronet_ErrorPtr = *mut c_void;
pub type Cronet_ClientContext = *mut c_void;
pub type Cronet_String = *const c_char;

pub const RESULT_SUCCESS: c_int = 0;
pub const HTTP_CACHE_MODE_DISK: c_int = 3;

// Callback function pointer types
pub type ExecuteFunc = unsafe extern "C" fn(Cronet_ExecutorPtr, Cronet_RunnablePtr);
pub type OnRedirectReceivedFunc = unsafe extern "C" fn(
    Cronet_UrlRequestCallbackPtr, Cronet_UrlRequestPtr, Cronet_UrlResponseInfoPtr, Cronet_String,
);
pub type OnResponseStartedFunc = unsafe extern "C" fn(
    Cronet_UrlRequestCallbackPtr, Cronet_UrlRequestPtr, Cronet_UrlResponseInfoPtr,
);
pub type OnReadCompletedFunc = unsafe extern "C" fn(
    Cronet_UrlRequestCallbackPtr, Cronet_UrlRequestPtr, Cronet_UrlResponseInfoPtr,
    Cronet_BufferPtr, u64,
);
pub type OnSucceededFunc = unsafe extern "C" fn(
    Cronet_UrlRequestCallbackPtr, Cronet_UrlRequestPtr, Cronet_UrlResponseInfoPtr,
);
pub type OnFailedFunc = unsafe extern "C" fn(
    Cronet_UrlRequestCallbackPtr, Cronet_UrlRequestPtr, Cronet_UrlResponseInfoPtr,
    Cronet_ErrorPtr,
);
pub type OnCanceledFunc = unsafe extern "C" fn(
    Cronet_UrlRequestCallbackPtr, Cronet_UrlRequestPtr, Cronet_UrlResponseInfoPtr,
);

extern "C" {
    // Engine params
    pub fn Cronet_EngineParams_Create() -> Cronet_EngineParamsPtr;
    pub fn Cronet_EngineParams_Destroy(params: Cronet_EngineParamsPtr);
    pub fn Cronet_EngineParams_enable_http2_set(params: Cronet_EngineParamsPtr, v: bool);
    pub fn Cronet_EngineParams_enable_quic_set(params: Cronet_EngineParamsPtr, v: bool);
    pub fn Cronet_EngineParams_enable_brotli_set(params: Cronet_EngineParamsPtr, v: bool);
    pub fn Cronet_EngineParams_user_agent_set(params: Cronet_EngineParamsPtr, ua: Cronet_String);
    pub fn Cronet_EngineParams_storage_path_set(params: Cronet_EngineParamsPtr, p: Cronet_String);
    pub fn Cronet_EngineParams_http_cache_mode_set(params: Cronet_EngineParamsPtr, mode: c_int);
    pub fn Cronet_EngineParams_http_cache_max_size_set(params: Cronet_EngineParamsPtr, size: i64);
    pub fn Cronet_EngineParams_enable_check_result_set(params: Cronet_EngineParamsPtr, v: bool);

    // Engine
    pub fn Cronet_Engine_Create() -> Cronet_EnginePtr;
    pub fn Cronet_Engine_Destroy(engine: Cronet_EnginePtr);
    pub fn Cronet_Engine_Shutdown(engine: Cronet_EnginePtr) -> c_int;
    pub fn Cronet_Engine_StartWithParams(engine: Cronet_EnginePtr, params: Cronet_EngineParamsPtr) -> c_int;

    // Executor
    pub fn Cronet_Executor_CreateWith(func: ExecuteFunc) -> Cronet_ExecutorPtr;
    pub fn Cronet_Executor_Destroy(executor: Cronet_ExecutorPtr);

    // Runnable
    pub fn Cronet_Runnable_Run(runnable: Cronet_RunnablePtr);
    pub fn Cronet_Runnable_Destroy(runnable: Cronet_RunnablePtr);

    // Buffer
    pub fn Cronet_Buffer_Create() -> Cronet_BufferPtr;
    pub fn Cronet_Buffer_Destroy(buffer: Cronet_BufferPtr);
    pub fn Cronet_Buffer_InitWithAlloc(buffer: Cronet_BufferPtr, size: u64);
    pub fn Cronet_Buffer_GetData(buffer: Cronet_BufferPtr) -> *mut c_void;
    pub fn Cronet_Buffer_GetSize(buffer: Cronet_BufferPtr) -> u64;

    // HttpHeader
    pub fn Cronet_HttpHeader_Create() -> Cronet_HttpHeaderPtr;
    pub fn Cronet_HttpHeader_Destroy(header: Cronet_HttpHeaderPtr);
    pub fn Cronet_HttpHeader_name_set(header: Cronet_HttpHeaderPtr, name: Cronet_String);
    pub fn Cronet_HttpHeader_value_set(header: Cronet_HttpHeaderPtr, value: Cronet_String);

    // UrlRequestParams
    pub fn Cronet_UrlRequestParams_Create() -> Cronet_UrlRequestParamsPtr;
    pub fn Cronet_UrlRequestParams_Destroy(params: Cronet_UrlRequestParamsPtr);
    pub fn Cronet_UrlRequestParams_http_method_set(params: Cronet_UrlRequestParamsPtr, method: Cronet_String);
    pub fn Cronet_UrlRequestParams_request_headers_add(params: Cronet_UrlRequestParamsPtr, header: Cronet_HttpHeaderPtr);

    // UrlRequestCallback
    pub fn Cronet_UrlRequestCallback_CreateWith(
        on_redirect: OnRedirectReceivedFunc,
        on_response_started: OnResponseStartedFunc,
        on_read_completed: OnReadCompletedFunc,
        on_succeeded: OnSucceededFunc,
        on_failed: OnFailedFunc,
        on_canceled: OnCanceledFunc,
    ) -> Cronet_UrlRequestCallbackPtr;
    pub fn Cronet_UrlRequestCallback_Destroy(cb: Cronet_UrlRequestCallbackPtr);
    pub fn Cronet_UrlRequestCallback_SetClientContext(cb: Cronet_UrlRequestCallbackPtr, ctx: Cronet_ClientContext);
    pub fn Cronet_UrlRequestCallback_GetClientContext(cb: Cronet_UrlRequestCallbackPtr) -> Cronet_ClientContext;

    // UrlRequest
    pub fn Cronet_UrlRequest_Create() -> Cronet_UrlRequestPtr;
    pub fn Cronet_UrlRequest_Destroy(request: Cronet_UrlRequestPtr);
    pub fn Cronet_UrlRequest_InitWithParams(
        request: Cronet_UrlRequestPtr,
        engine: Cronet_EnginePtr,
        url: Cronet_String,
        params: Cronet_UrlRequestParamsPtr,
        callback: Cronet_UrlRequestCallbackPtr,
        executor: Cronet_ExecutorPtr,
    ) -> c_int;
    pub fn Cronet_UrlRequest_Start(request: Cronet_UrlRequestPtr) -> c_int;
    pub fn Cronet_UrlRequest_Read(request: Cronet_UrlRequestPtr, buffer: Cronet_BufferPtr) -> c_int;
    pub fn Cronet_UrlRequest_FollowRedirect(request: Cronet_UrlRequestPtr) -> c_int;

    // UrlResponseInfo
    pub fn Cronet_UrlResponseInfo_http_status_code_get(info: Cronet_UrlResponseInfoPtr) -> i32;
}

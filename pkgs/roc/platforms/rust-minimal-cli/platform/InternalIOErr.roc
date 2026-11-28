module [
    IOErr,
    IOErrFromHost,
    handle_err,
]
IOErr : [
    NotFound,
    PermissionDenied,
    BrokenPipe,
    AlreadyExists,
    Interrupted,
    Unsupported,
    OutOfMemory,
    Other Str,
]
IOErrFromHost : {
    tag : [
        EndOfFile,
        NotFound,
        PermissionDenied,
        BrokenPipe,
        AlreadyExists,
        Interrupted,
        Unsupported,
        OutOfMemory,
        Other,
    ],
    msg : Str,
}
handle_err : IOErrFromHost -> IOErr
handle_err = |{ tag, msg }|
    when tag is
        NotFound -> NotFound
        PermissionDenied -> PermissionDenied
        BrokenPipe -> BrokenPipe
        AlreadyExists -> AlreadyExists
        Interrupted -> Interrupted
        Unsupported -> Unsupported
        OutOfMemory -> OutOfMemory
        Other | EndOfFile -> Other(msg)

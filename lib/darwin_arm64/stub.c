// Stub for Cronet_CreateCertVerifierWithPublicKeySHA256 which exists in the
// cronet-go Go bindings but was not included in the prebuilt static library.
// This function is never called by Wick — it's only needed to satisfy the linker.
// If it is ever called, abort immediately to surface the mismatch.

#include <stddef.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>

void* Cronet_CreateCertVerifierWithPublicKeySHA256(
    const uint8_t** hashes,
    size_t hash_count) {
    (void)hashes;
    (void)hash_count;
    fprintf(stderr,
            "Fatal: Cronet_CreateCertVerifierWithPublicKeySHA256 stub was called; "
            "ensure libcronet.a matches the cronet-go bindings.\n");
    abort();
}

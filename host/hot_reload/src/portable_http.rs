use std::io::Read;

use aimer_anteros::{
    CapabilityDecoder, CapabilityDescriptor, CapabilityEncoder, CapabilityError,
    CapabilityGeneration, CapabilityLimits, CapabilityProvider, CapabilityRegistry,
    CapabilityResult, CapabilityStagingClass, PORTABLE_HTTP_CAPABILITY_ABI_MAJOR,
    PORTABLE_HTTP_MAX_REQUEST_BYTES, PORTABLE_HTTP_MAX_RESPONSE_BYTES, PORTABLE_HTTP_METHOD_GET,
    portable_http_capability_id, portable_http_contract_fingerprint, portable_http_max_body_bytes,
};

/// Registers the bounded HTTP provider available to generated portable guests.
pub(crate) fn default_registry() -> CapabilityRegistry {
    let mut registry = CapabilityRegistry::new(1);
    registry
        .register_with_staging(PortableHttpProvider, CapabilityStagingClass::ReadOnly)
        .expect("the built-in portable HTTP capability must register once");
    registry
}

struct PortableHttpProvider;

impl CapabilityProvider for PortableHttpProvider {
    fn descriptor(&self) -> CapabilityDescriptor {
        CapabilityDescriptor::new(
            portable_http_capability_id(),
            PORTABLE_HTTP_CAPABILITY_ABI_MAJOR,
            portable_http_contract_fingerprint(),
            CapabilityLimits::new(
                PORTABLE_HTTP_MAX_REQUEST_BYTES,
                PORTABLE_HTTP_MAX_RESPONSE_BYTES,
            ),
        )
    }

    fn invoke(
        &self,
        _generation: CapabilityGeneration,
        method_id: u32,
        request: &[u8],
        response_limit: u32,
    ) -> CapabilityResult<Vec<u8>> {
        if method_id != PORTABLE_HTTP_METHOD_GET {
            return Err(CapabilityError::Unsupported);
        }
        let mut decoder = CapabilityDecoder::new_request(request, PORTABLE_HTTP_MAX_REQUEST_BYTES)?;
        let url = decoder.read_string()?;
        decoder.finish()?;

        let response = reqwest::blocking::get(&url).map_err(|_| CapabilityError::Unavailable)?;
        let status = response.status().as_u16();
        let mut body = Vec::new();
        response
            .take(u64::from(portable_http_max_body_bytes()) + 1)
            .read_to_end(&mut body)
            .map_err(|_| CapabilityError::Unavailable)?;
        if body.len() > portable_http_max_body_bytes() as usize {
            return Err(CapabilityError::LimitExceeded);
        }

        let mut encoder =
            CapabilityEncoder::new(response_limit.min(PORTABLE_HTTP_MAX_RESPONSE_BYTES));
        encoder.write_u16(status)?;
        encoder.write_bytes(&body)?;
        Ok(encoder.into_bytes())
    }
}

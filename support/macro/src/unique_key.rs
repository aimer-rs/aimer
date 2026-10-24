use syn::LitStr;
use syn::parse::Parse;

pub struct UniqueKeyInput {
    pub(crate) prefix: Option<LitStr>,
}

impl Parse for UniqueKeyInput {
    fn parse(input: syn::parse::ParseStream) -> syn::Result<Self> {
        if input.is_empty() {
            Ok(Self { prefix: None })
        } else {
            Ok(Self {
                prefix: Some(input.parse()?),
            })
        }
    }
}

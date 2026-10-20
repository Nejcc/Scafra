use scafra::bean;

pub struct UnmanagedPool;

#[bean]
pub async fn pool() -> Result<UnmanagedPool, std::io::Error> {
    Ok(UnmanagedPool)
}

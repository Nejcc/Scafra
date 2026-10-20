use scafra::{bean, service};

pub struct Database;

#[bean]
pub async fn database() -> Result<Database, std::io::Error> {
    Ok(Database)
}

#[service]
pub struct Repository {
    database: Database,
}

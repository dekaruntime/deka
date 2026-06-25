mod commands;
mod config;
mod generate;
mod migrate;

pub use commands::register;

#[cfg(test)]
mod tests;

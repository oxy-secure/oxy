pub fn serialize<S>(input: &Option<Vec<u8>>, serializer: S) -> Result<S::Ok, S::Error>
where
    S: serde::ser::Serializer,
{
    serializer.serialize_str(&data_encoding::BASE32_NOPAD.encode(&input.as_ref().unwrap()[..]))
}

pub fn deserialize<'de, D>(input: D) -> Result<Option<Vec<u8>>, D::Error>
where
    D: serde::de::Deserializer<'de>,
{
    let val: &'de str = serde::de::Deserialize::deserialize(input)?;
    Ok(Some(
        data_encoding::BASE32_NOPAD
            .decode(val.as_bytes())
            .map_err(|_| <D::Error as ::serde::de::Error>::custom("invalid base32"))?,
    ))
}

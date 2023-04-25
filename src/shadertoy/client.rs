use super::errors::*;
use super::types::*;
use error_chain::bail;
use reqwest;
use serde::{Deserialize, Serialize};
use serde_json;
use std;
use std::io::Cursor;
use std::io::Write;
use std::str::FromStr;

#[derive(Serialize, Deserialize, Debug, PartialEq, Copy, Clone)]
pub enum SearchSortOrder {
    Name,
    Love,
    Popular,
    Newest,
    Hot,
}

#[derive(Serialize, Deserialize, Debug, PartialEq, Copy, Clone)]
pub enum SearchFilter {
    Vr,
    SoundOutput,
    SoundInput,
    Webcam,
    MultiPass,
    MusicStream,
}

/// Search parameters for `Client::search`.
#[derive(Serialize, Deserialize, Debug, PartialEq, Clone)]
pub struct SearchParams<'a> {
    /// Search string, set as empty to get ALL shadertoys.
    pub string: &'a str,
    /// Sort order of resulting list of shaders.
    pub sort_order: SearchSortOrder,
    /// Inclusion filters, only the shadertoys matching this filter will be included in the result.
    pub filters: Vec<SearchFilter>,
}

/// Client for issuing queries against the Shadertoy API and database
pub struct Client {
    pub api_key: String,
    pub rest_client: reqwest::Client,
}

impl FromStr for SearchSortOrder {
    type Err = ();

    fn from_str(s: &str) -> std::result::Result<SearchSortOrder, ()> {
        match s {
            "Name" => Ok(SearchSortOrder::Name),
            "Love" => Ok(SearchSortOrder::Love),
            "Popular" => Ok(SearchSortOrder::Popular),
            "Newest" => Ok(SearchSortOrder::Newest),
            "Hot" => Ok(SearchSortOrder::Hot),
            _ => Err(()),
        }
    }
}

impl FromStr for SearchFilter {
    type Err = ();

    fn from_str(s: &str) -> std::result::Result<SearchFilter, ()> {
        match s {
            "VR" => Ok(SearchFilter::Vr),
            "SoundOutput" => Ok(SearchFilter::SoundOutput),
            "SoundInput" => Ok(SearchFilter::SoundInput),
            "Webcam" => Ok(SearchFilter::Webcam),
            "MultiPass" => Ok(SearchFilter::MultiPass),
            "MusicStream" => Ok(SearchFilter::MusicStream),
            _ => Err(()),
        }
    }
}

impl Client {
    /// Create a new client.
    /// This requires sending in an API key, one can generate one on https://www.shadertoy.com/profile
    pub fn new(api_key: &str) -> Client {
        Client {
            api_key: api_key.to_string(),
            rest_client: reqwest::Client::new(),
        }
    }

    /// Issues a search query for shadertoys.
    /// If the query is successful a list of shader ids will be returned,
    /// which can be used with `get_shader`.
    ///
    /// # Examples
    ///
    /// ```
    /// # fn main() {
    /// let client = shadertoy::Client::new("Bd8tWD"); // insert your own API key here
    /// let search_params = shadertoy::SearchParams {
    /// 	string: "car",
    ///     sort_order: shadertoy::SearchSortOrder::Love,
    ///     filters: vec![],
    /// };
    /// match client.search(&search_params) {
    /// 	Ok(shader_ids) => println!("\"Car\" shadertoys: {:?}", shader_ids),
    /// 	Err(err) => println!("Search failed: {}", err),
    /// }
    /// # }
    /// ```
    pub async fn search(&self, params: &SearchParams<'_>) -> Result<Vec<String>> {
        let query_str = format!(
            "https://www.shadertoy.com/api/v1/shaders{}?sort={}&{}key={}",
            if params.string.is_empty() {
                "".to_string()
            } else {
                format!("/query/{}", params.string)
            },
            format!("{:?}", params.sort_order).to_lowercase(),
            params
                .filters
                .iter()
                .map(|f| format!("filter={:?}&", f).to_lowercase())
                .collect::<String>(),
            self.api_key
        );

        let json_str = self
            .rest_client
            .get(&query_str)
            .send()
            .await?
            .text()
            .await?;

        #[derive(Serialize, Deserialize, Debug)]
        #[serde(deny_unknown_fields)]
        struct SearchResult {
            #[serde(default)]
            #[serde(rename = "Error")]
            error: String,

            #[serde(default)]
            #[serde(rename = "Shaders")]
            shaders: u64,

            #[serde(default)]
            #[serde(rename = "Results")]
            results: Vec<String>,
        }

        match serde_json::from_str::<SearchResult>(&json_str) {
            Ok(r) => {
                if !r.error.is_empty() {
                    bail!("Shadertoy REST search query returned error: {}", r.error);
                }
                Ok(r.results)
            }
            Err(err) => {
                Err(Error::from(err)).chain_err(|| "JSON parsing of Shadertoy search result failed")
            }
        }
    }

    /// Retrives a shader given an id.
    pub async fn get_shader(&self, shader_id: &str, save: Option<&str>) -> Result<Shader> {
        let json = self
            .rest_client
            .get(&format!(
                "https://www.shadertoy.com/api/v1/shaders/{}?key={}",
                shader_id, self.api_key
            ))
            .send()
            .await?
            .json::<ShaderRoot>()
            .await?;

        #[derive(Serialize, Deserialize, Debug)]
        #[serde(deny_unknown_fields)]
        struct ShaderRoot {
            #[serde(default)]
            #[serde(rename = "Error")]
            error: String,

            #[serde(rename = "Shader")]
            shader: Shader,
        }

        if !json.error.is_empty() {
            bail!("Shadertoy REST shader query returned error: {}", json.error);
        }

        if let Some(p) = save {
            let st = serde_json::to_vec_pretty(&json.shader).unwrap();
            let mut f = std::fs::File::create(p).unwrap();
            f.write_all(&st).unwrap();
        }

        Ok(json.shader)
    }

    pub async fn get_resource(&self, resource: &str) -> Result<Vec<u8>> {
        let data = self
            .rest_client
            .get(&format!(
                "https://www.shadertoy.com/{}?key={}",
                resource, self.api_key
            ))
            .send()
            .await?
            .bytes()
            .await?;

        Ok(data.to_vec())
    }

    pub async fn get_png(&self, resource: &str, is_cube: bool) -> Result<(Vec<u8>, (u32, u32))> {
        use image::io::Reader as ImageReader;

        let bytes = self.get_resource(resource).await?;
        let img2 = ImageReader::new(Cursor::new(bytes))
            .with_guessed_format()?
            .decode()?;

        let size = (img2.width(), img2.height());
        let mut raw = img2.into_rgba8().into_raw();

        if is_cube {
            let dot_idx = resource.rfind('.').unwrap();
            let (start, end) = resource.split_at(dot_idx);
            for i in 1..6 {
                println!("Getting more cube things {}", i);

                let url = format!("{}_{}{}", start, i, end);

                let bytes = self.get_resource(&url).await?;
                let img2 = ImageReader::new(Cursor::new(bytes))
                    .with_guessed_format()?
                    .decode()?;

                raw.extend_from_slice(&img2.into_rgba8().as_raw());
            }

            Ok((raw, size))
        } else {
            Ok((raw, size))
        }

        // let mut reader = png::Decoder::new(Cursor::new(bytes)).read_info()?;
        //
        // let mut buf = vec![0; reader.output_buffer_size()];
        // let info = reader.next_frame(&mut buf).unwrap();
        //
        // let mut tmp = vec![0; reader.output_buffer_size()];
        // let mut i = 1;
        // while reader.next_frame(&mut tmp).is_ok() {
        //     i += 1;
        // }
        // println!(
        //     "expected {} * {} * 4 = {} = {}",
        //     info.width,
        //     info.height,
        //     info.width * info.height * 4,
        //     reader.output_buffer_size()
        // );
        // println!("found {i} frames!");
        //
        // Ok((buf, info))
    }
}

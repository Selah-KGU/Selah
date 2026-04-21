use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
pub struct WeatherData {
    pub temperature: i32,
    #[serde(rename = "weatherCode")]
    pub weather_code: i32,
    pub humidity: i32,
    #[serde(rename = "windSpeed")]
    pub wind_speed: i32,
    pub tomorrow: Option<WeatherTomorrow>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct WeatherTomorrow {
    #[serde(rename = "tempMax")]
    pub temp_max: i32,
    #[serde(rename = "tempMin")]
    pub temp_min: i32,
    #[serde(rename = "weatherCode")]
    pub weather_code: i32,
}

#[tauri::command]
pub async fn fetch_weather() -> Result<WeatherData, String> {
    let url = "https://api.open-meteo.com/v1/forecast?latitude=34.7383&longitude=135.3416&current=temperature_2m,weather_code,relative_humidity_2m,wind_speed_10m&daily=weather_code,temperature_2m_max,temperature_2m_min&timezone=Asia%2FTokyo&forecast_days=2";
    let resp: serde_json::Value = reqwest::get(url)
        .await
        .map_err(|e| format!("天気API接続失敗: {}", e))?
        .json()
        .await
        .map_err(|e| format!("天気API解析失敗: {}", e))?;

    let current = &resp["current"];
    let daily = &resp["daily"];

    let tomorrow = if daily["time"].as_array().is_some_and(|a| a.len() >= 2) {
        Some(WeatherTomorrow {
            temp_max: daily["temperature_2m_max"][1]
                .as_f64()
                .unwrap_or(0.0)
                .round() as i32,
            temp_min: daily["temperature_2m_min"][1]
                .as_f64()
                .unwrap_or(0.0)
                .round() as i32,
            weather_code: daily["weather_code"][1].as_i64().unwrap_or(0) as i32,
        })
    } else {
        None
    };

    Ok(WeatherData {
        temperature: current["temperature_2m"].as_f64().unwrap_or(0.0).round() as i32,
        weather_code: current["weather_code"].as_i64().unwrap_or(0) as i32,
        humidity: current["relative_humidity_2m"].as_i64().unwrap_or(0) as i32,
        wind_speed: current["wind_speed_10m"].as_f64().unwrap_or(0.0).round() as i32,
        tomorrow,
    })
}

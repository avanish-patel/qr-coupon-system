// Cargo.toml dependencies:
/*
[package]
name = "qr-coupon-system"
version = "0.1.0"
edition = "2021"

[dependencies]
axum = "0.7"
tokio = { version = "1.40", features = ["full"] }
tower = "0.4"
tower-http = { version = "0.5", features = ["fs", "cors"] }
serde = { version = "1.0", features = ["derive"] }
serde_json = "1.0"
sqlx = { version = "0.8", features = ["runtime-tokio-rustls", "sqlite"] }
qrcode = "0.14"
image = "0.25"
base64 = "0.22"
uuid = { version = "1.0", features = ["v4"] }
chrono = { version = "0.4", features = ["serde"] }

[profile.release]
opt-level = "z"
lto = true
codegen-units = 1
strip = true
*/

// src/main.rs
use axum::{
    extract::{Json, State, Query},
    http::StatusCode,
    response::{Html, IntoResponse},
    routing::{get, post},
    Router,
};
use serde::{Deserialize, Serialize};
use sqlx::{sqlite::SqlitePool, FromRow};
use std::{sync::Arc, str::FromStr};
use tower_http::cors::CorsLayer;
use qrcode::QrCode;
use image::Luma;
use base64::{Engine as _, engine::general_purpose};
use chrono::{NaiveDate, Utc};

// Application state
struct AppState {
    db: SqlitePool,
}

// Database models
#[derive(FromRow, Serialize, Clone)]
struct Coupon {
    id: i64,
    code: String,
    batch_name: Option<String>,
    description: Option<String>,
    valid_from: Option<String>,
    valid_until: Option<String>,
    created_at: String,
    used_at: Option<String>,
    is_used: bool,
}

// API request/response types
#[derive(Deserialize)]
struct LoginRequest {
    password: String,
}

#[derive(Serialize)]
struct LoginResponse {
    success: bool,
    role: Option<String>,
    message: String,
}

#[derive(Deserialize)]
struct GenerateRequest {
    count: u32,
    batch_name: Option<String>,
    description: Option<String>,
    valid_from: Option<String>,
    valid_until: Option<String>,
}

#[derive(Serialize)]
struct QrCodeData {
    code: String,
    qr_image: String,
    batch_name: Option<String>,
    description: Option<String>,
    valid_until: Option<String>,
}

#[derive(Serialize)]
struct GenerateResponse {
    qr_codes: Vec<QrCodeData>,
}

#[derive(Deserialize)]
struct ValidateRequest {
    code: String,
}

#[derive(Serialize)]
struct ValidateResponse {
    success: bool,
    message: String,
    coupon: Option<CouponDetails>,
}

#[derive(Serialize)]
struct CouponDetails {
    description: Option<String>,
    batch_name: Option<String>,
    valid_until: Option<String>,
}

#[derive(Deserialize)]
struct CouponQuery {
    search: Option<String>,
    batch: Option<String>,
    status: Option<String>,
}

#[derive(Serialize)]
struct StatsResponse {
    total: i64,
    active: i64,
    used: i64,
    expired: i64,
}

#[tokio::main]
async fn main() {
    // Initialize database - use data directory for persistence
    let db_path = std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| "sqlite:/app/data/coupons.db".to_string());
    
    // Parse connection options and enable create_if_missing
    let options = sqlx::sqlite::SqliteConnectOptions::from_str(&db_path)
        .expect("Invalid database URL")
        .create_if_missing(true);
    
    let db = SqlitePool::connect_with(options)
        .await
        .expect("Failed to connect to database");

    // Create tables with new schema
    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS coupons (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            code TEXT UNIQUE NOT NULL,
            batch_name TEXT,
            description TEXT,
            valid_from TEXT,
            valid_until TEXT,
            created_at DATETIME DEFAULT CURRENT_TIMESTAMP,
            used_at DATETIME,
            is_used BOOLEAN DEFAULT 0
        )
        "#,
    )
    .execute(&db)
    .await
    .expect("Failed to create table");

    // Create indexes for better performance
    sqlx::query("CREATE INDEX IF NOT EXISTS idx_batch ON coupons(batch_name)")
        .execute(&db)
        .await
        .ok();
    
    sqlx::query("CREATE INDEX IF NOT EXISTS idx_used ON coupons(is_used)")
        .execute(&db)
        .await
        .ok();

    let state = Arc::new(AppState { db });

    // Build router
    let app = Router::new()
        .route("/", get(serve_index))
        .route("/api/login", post(login))
        .route("/api/generate", post(generate_qr_codes))
        .route("/api/validate", post(validate_qr_code))
        .route("/api/coupons", get(list_coupons))
        .route("/api/stats", get(get_stats))
        .route("/api/batches", get(get_batches))
        .layer(CorsLayer::permissive())
        .with_state(state);

    let listener = tokio::net::TcpListener::bind("0.0.0.0:3000")
        .await
        .unwrap();
    
    println!("Server running on http://0.0.0.0:3000");
    axum::serve(listener, app).await.unwrap();
}

async fn serve_index() -> impl IntoResponse {
    Html(include_str!("../static/index.html"))
}

async fn login(Json(payload): Json<LoginRequest>) -> Json<LoginResponse> {
    // Simple password check (in production, use proper password hashing)
    let response = if payload.password == "1111" {
        LoginResponse {
            success: true,
            role: Some("admin".to_string()),
            message: "Login successful".to_string(),
        }
    } else if payload.password == "2222" {
        LoginResponse {
            success: true,
            role: Some("validator".to_string()),
            message: "Login successful".to_string(),
        }
    } else {
        LoginResponse {
            success: false,
            role: None,
            message: "Invalid password".to_string(),
        }
    };

    Json(response)
}

async fn generate_qr_codes(
    State(state): State<Arc<AppState>>,
    Json(payload): Json<GenerateRequest>,
) -> Result<Json<GenerateResponse>, StatusCode> {
    let count = payload.count.min(100); // Limit to 100 per request
    let mut qr_codes = Vec::new();

    for _ in 0..count {
        let code = uuid::Uuid::new_v4().to_string();
        
        // Insert into database with new fields
        let result = sqlx::query(
            "INSERT INTO coupons (code, batch_name, description, valid_from, valid_until) VALUES (?, ?, ?, ?, ?)"
        )
            .bind(&code)
            .bind(&payload.batch_name)
            .bind(&payload.description)
            .bind(&payload.valid_from)
            .bind(&payload.valid_until)
            .execute(&state.db)
            .await;

        if result.is_err() {
            continue;
        }

        // Generate QR code
        if let Ok(qr) = QrCode::new(&code) {
            let image = qr.render::<Luma<u8>>().build();
            let mut buffer = Vec::new();
            
            if let Ok(_) = image::DynamicImage::ImageLuma8(image)
                .write_to(&mut std::io::Cursor::new(&mut buffer), image::ImageFormat::Png)
            {
                let base64_image = general_purpose::STANDARD.encode(&buffer);
                qr_codes.push(QrCodeData {
                    code,
                    qr_image: format!("data:image/png;base64,{}", base64_image),
                    batch_name: payload.batch_name.clone(),
                    description: payload.description.clone(),
                    valid_until: payload.valid_until.clone(),
                });
            }
        }
    }

    Ok(Json(GenerateResponse { qr_codes }))
}

async fn validate_qr_code(
    State(state): State<Arc<AppState>>,
    Json(payload): Json<ValidateRequest>,
) -> Json<ValidateResponse> {
    // Check if coupon exists and is not used
    let result = sqlx::query_as::<_, Coupon>(
        "SELECT * FROM coupons WHERE code = ? AND is_used = 0"
    )
        .bind(&payload.code)
        .fetch_optional(&state.db)
        .await;

    match result {
        Ok(Some(coupon)) => {
            // Check if expired
            let today = Utc::now().date_naive();
            if let Some(valid_until) = &coupon.valid_until {
                if let Ok(expiry_date) = NaiveDate::parse_from_str(valid_until, "%Y-%m-%d") {
                    if today > expiry_date {
                        return Json(ValidateResponse {
                            success: false,
                            message: format!("✗ Coupon expired on {}", valid_until),
                            coupon: None,
                        });
                    }
                }
            }

            // Check if not yet valid
            if let Some(valid_from) = &coupon.valid_from {
                if let Ok(start_date) = NaiveDate::parse_from_str(valid_from, "%Y-%m-%d") {
                    if today < start_date {
                        return Json(ValidateResponse {
                            success: false,
                            message: format!("✗ Coupon not valid until {}", valid_from),
                            coupon: None,
                        });
                    }
                }
            }

            // Mark as used
            let update_result = sqlx::query(
                "UPDATE coupons SET is_used = 1, used_at = CURRENT_TIMESTAMP WHERE code = ?"
            )
                .bind(&payload.code)
                .execute(&state.db)
                .await;

            if update_result.is_ok() {
                Json(ValidateResponse {
                    success: true,
                    message: "✓ Coupon validated successfully!".to_string(),
                    coupon: Some(CouponDetails {
                        description: coupon.description,
                        batch_name: coupon.batch_name,
                        valid_until: coupon.valid_until,
                    }),
                })
            } else {
                Json(ValidateResponse {
                    success: false,
                    message: "Error redeeming coupon".to_string(),
                    coupon: None,
                })
            }
        }
        Ok(None) => Json(ValidateResponse {
            success: false,
            message: "✗ Invalid coupon code or already used".to_string(),
            coupon: None,
        }),
        Err(_) => Json(ValidateResponse {
            success: false,
            message: "Database error".to_string(),
            coupon: None,
        }),
    }
}

async fn list_coupons(
    State(state): State<Arc<AppState>>,
    Query(query): Query<CouponQuery>,
) -> Result<Json<Vec<Coupon>>, StatusCode> {
    let mut sql = "SELECT * FROM coupons WHERE 1=1".to_string();
    
    if let Some(batch) = &query.batch {
        sql.push_str(&format!(" AND batch_name = '{}'", batch));
    }
    
    if let Some(status) = &query.status {
        match status.as_str() {
            "active" => sql.push_str(" AND is_used = 0"),
            "used" => sql.push_str(" AND is_used = 1"),
            _ => {}
        }
    }
    
    if let Some(search) = &query.search {
        sql.push_str(&format!(" AND (code LIKE '%{}%' OR description LIKE '%{}%' OR batch_name LIKE '%{}%')", search, search, search));
    }
    
    sql.push_str(" ORDER BY created_at DESC");
    
    let coupons = sqlx::query_as::<_, Coupon>(&sql)
        .fetch_all(&state.db)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    Ok(Json(coupons))
}

async fn get_stats(State(state): State<Arc<AppState>>) -> Result<Json<StatsResponse>, StatusCode> {
    let total: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM coupons")
        .fetch_one(&state.db)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    
    let used: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM coupons WHERE is_used = 1")
        .fetch_one(&state.db)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    
    let today = Utc::now().date_naive().format("%Y-%m-%d").to_string();
    let expired: (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM coupons WHERE is_used = 0 AND valid_until < ?"
    )
        .bind(&today)
        .fetch_one(&state.db)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    
    let active = total.0 - used.0 - expired.0;
    
    Ok(Json(StatsResponse {
        total: total.0,
        active,
        used: used.0,
        expired: expired.0,
    }))
}

async fn get_batches(State(state): State<Arc<AppState>>) -> Result<Json<Vec<String>>, StatusCode> {
    let batches: Vec<(Option<String>,)> = sqlx::query_as(
        "SELECT DISTINCT batch_name FROM coupons WHERE batch_name IS NOT NULL ORDER BY batch_name"
    )
        .fetch_all(&state.db)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    
    let batch_names: Vec<String> = batches
        .into_iter()
        .filter_map(|(name,)| name)
        .collect();
    
    Ok(Json(batch_names))
}

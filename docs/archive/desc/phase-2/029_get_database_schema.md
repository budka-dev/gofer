# Feature: get_database_schema - Database Intelligence

**ID:** PHASE2-029  
**Priority:** 🔥🔥🔥 High  
**Effort:** 4 дня  
**Status:** Not Started  
**Phase:** 2 (Database Intelligence)

---

## 📋 Описание

Извлечение и анализ database schema: tables, columns, types, constraints, indexes, foreign keys. Поддержка PostgreSQL, MySQL, SQLite.

### Проблема

```
AI: "Какие таблицы есть в БД?"
→ Нет visibility в database schema

Developer: "Какие есть indexes?"
→ Schema не документирована в коде
```

### Решение

```typescript
const schema = await gofer.get_database_schema();

// Returns:
// Table: users
//   - id: SERIAL PRIMARY KEY
//   - email: VARCHAR(255) UNIQUE NOT NULL
//   - created_at: TIMESTAMP
//   Indexes: idx_users_email (BTREE)
//   Foreign keys: none
// Table: orders
//   - user_id → users.id
//   ...
```

---

## 🎯 Goals & Non-Goals

### Goals
- ✅ Extract full schema (tables, columns, types)
- ✅ Indexes with usage statistics
- ✅ Foreign key relationships
- ✅ Support PostgreSQL, MySQL, SQLite

### Non-Goals
- ❌ Не schema migration
- ❌ Не query optimization (separate tool)

---

## 🔧 API Specification

```json
{
  "name": "get_database_schema",
  "description": "Получить схему базы данных",
  "inputSchema": {
    "type": "object",
    "properties": {
      "connection": {
        "type": "string",
        "description": "Optional: specific connection"
      }
    }
  }
}
```

### Response Schema

```rust
#[derive(Serialize)]
pub struct DatabaseSchema {
    pub tables: Vec<Table>,
    pub relationships: Vec<ForeignKeyRelationship>,
}

#[derive(Serialize)]
pub struct Table {
    pub name: String,
    pub columns: Vec<Column>,
    pub indexes: Vec<Index>,
    pub constraints: Vec<Constraint>,
}

#[derive(Serialize)]
pub struct Column {
    pub name: String,
    pub data_type: String,
    pub nullable: bool,
    pub default_value: Option<String>,
}

#[derive(Serialize)]
pub struct Index {
    pub name: String,
    pub columns: Vec<String>,
    pub index_type: String,
    pub unique: bool,
}

#[derive(Serialize)]
pub struct ForeignKeyRelationship {
    pub from_table: String,
    pub from_column: String,
    pub to_table: String,
    pub to_column: String,
}
```

---

## 💻 Implementation

```rust
pub async fn get_database_schema(
    connection: Option<&str>
) -> Result<DatabaseSchema> {
    let db_url = connection.unwrap_or(&env::var("DATABASE_URL")?);
    
    // Detect database type
    let db_type = detect_database_type(db_url)?;
    
    match db_type {
        DatabaseType::PostgreSQL => get_postgres_schema(db_url).await,
        DatabaseType::MySQL => get_mysql_schema(db_url).await,
        DatabaseType::SQLite => get_sqlite_schema(db_url).await,
    }
}

async fn get_postgres_schema(url: &str) -> Result<DatabaseSchema> {
    let pool = PgPool::connect(url).await?;
    
    // Query information_schema
    let tables_query = r#"
        SELECT table_name
        FROM information_schema.tables
        WHERE table_schema = 'public'
    "#;
    
    let table_names: Vec<String> = sqlx::query_scalar(tables_query)
        .fetch_all(&pool)
        .await?;
    
    let mut tables = Vec::new();
    
    for table_name in table_names {
        // Get columns
        let columns = get_table_columns(&pool, &table_name).await?;
        
        // Get indexes
        let indexes = get_table_indexes(&pool, &table_name).await?;
        
        // Get constraints
        let constraints = get_table_constraints(&pool, &table_name).await?;
        
        tables.push(Table {
            name: table_name,
            columns,
            indexes,
            constraints,
        });
    }
    
    // Get relationships
    let relationships = get_foreign_keys(&pool).await?;
    
    Ok(DatabaseSchema {
        tables,
        relationships,
    })
}

async fn get_table_columns(
    pool: &PgPool,
    table_name: &str
) -> Result<Vec<Column>> {
    let query = r#"
        SELECT column_name, data_type, is_nullable, column_default
        FROM information_schema.columns
        WHERE table_name = $1
        ORDER BY ordinal_position
    "#;
    
    let rows = sqlx::query(query)
        .bind(table_name)
        .fetch_all(pool)
        .await?;
    
    let columns = rows.iter().map(|row| Column {
        name: row.get("column_name"),
        data_type: row.get("data_type"),
        nullable: row.get::<String, _>("is_nullable") == "YES",
        default_value: row.get("column_default"),
    }).collect();
    
    Ok(columns)
}

async fn get_table_indexes(
    pool: &PgPool,
    table_name: &str
) -> Result<Vec<Index>> {
    let query = r#"
        SELECT indexname, indexdef
        FROM pg_indexes
        WHERE tablename = $1
    "#;
    
    // Parse index definitions
    // ...
    
    todo!()
}
```

---

## 📈 Success Metrics

- ✅ Extracts complete schema
- ✅ Relationships accurate
- ⏱️ Response time < 5s

---

## ✅ Acceptance Criteria

- [ ] PostgreSQL support
- [ ] MySQL support
- [ ] SQLite support
- [ ] Foreign keys extracted
- [ ] Indexes listed
- [ ] All tests pass

---

**Status:** Ready for implementation  
**Last Updated:** 2026-02-16

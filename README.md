# Koiweb Framework

Koiweb is a micro web framework for Rust, inspired by Express.js. It aims to provide a simple, flexible, and performant way to build web applications.

## Features

- **Routing:** Define routes for different HTTP methods and paths, including dynamic path parameters.
- **Route Groups:** Group routes under a common URL prefix for better organization.
- **Middleware:** Easily add middleware to process requests before they reach the final handler.
- **Request/Response Objects:** Convenient abstractions for handling HTTP requests and crafting responses.
- **Error Handling:** Centralized error handling mechanism.
- **File Uploads:** Example of handling file uploads.

## Project Structure

- `src/lib.rs`: Contains the core framework logic (App, Router, Request, Response, Middleware, etc.).
- `src/main.rs`: An example application demonstrating how to use the framework.
- `public/`: Directory for serving static files (currently not implemented, but a planned feature).
- `uploads/`: Directory where uploaded files are stored.

## Getting Started

### Prerequisites

- Rust (latest stable version recommended)
- Cargo (comes with Rust)

### Building and Running

1.  **Clone the repository:**

    ```bash
    git clone https://github.com/KhaiNguyenHoang/koiweb-framework.git
    cd koiweb-framework
    ```

2.  **Run the example application:**
    ```bash
    cargo run
    ```
    The server will start on `http://127.0.0.1:8080` (or the port specified by the `PORT` environment variable).

### Usage Examples

#### Home Route

- **GET /**
  Returns "Home".

#### User Route with Path Parameter

- **GET /users/:id**
  Example: `GET /users/123`
  Returns a JSON object: `{"user_id": "123"}`

#### File Upload

- **POST /upload**
  Uploads a file to the `uploads/` directory. Files are limited to 10MB.
  Example using `curl`:
  ```bash
  curl -X POST -H "Content-Type: application/octet-stream" --data-binary "@your_file.txt" http://127.0.0.1:8080/upload
  ```

#### API Group Route

- **GET /api/v1/test**
  Returns a JSON object: `{"status": "ok"}`

## Configuration

The server's listening port can be configured using the `PORT` environment variable. If not set, it defaults to `8080`.

```bash
PORT=3000 cargo run
```

## Contributing

Contributions are welcome! Please open an issue or submit a pull request.

## License

This project is licensed under the MIT License.

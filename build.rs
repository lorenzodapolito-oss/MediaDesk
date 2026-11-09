//! Incorpora l'icona nell'eseguibile Windows (se non riesce, il programma si compila lo stesso).

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=assets/icon.ico");
    #[cfg(windows)]
    {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("assets/icon.ico");
        if let Err(e) = res.compile() {
            println!("cargo:warning=icona dell'exe non incorporata: {e}");
        }
    }
}

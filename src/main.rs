use slint::{ModelRc, VecModel};
use rick::data::FileItem;

slint::include_modules!();

fn main() -> Result<(), slint::PlatformError>  {
    let app: App = App::new()?;
    let files = get_files();
    let newfiles = files.iter().map(|e| UiFileItem { name: e.name.clone().into(), size: e.size as i32});
    let test = ModelRc::new(VecModel::from(newfiles.collect::<Vec<UiFileItem>>()));

    app.set_files(test);
    app.run()?;
    println!("Hello, world!");
    Ok(())
}

fn get_files() -> Vec<FileItem> {
    vec![
        FileItem { name: "file1".to_string(), size: 100},
        FileItem { name: "file2".to_string(), size: 100},
    ]
}



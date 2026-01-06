// use std::os::unix::raw::mode_t;

//use std::fs::File;

use iced::widget::{Column, column, text};
use iced::Element;
//use slint::{ModelRc, VecModel};
use rick::data::FileItem;

slint::include_modules!();

#[derive(Debug, Clone, Copy)]
enum ViewMode {
    Loading,
    Ready,
}


struct AppState {
    files: Vec<FileItem>,
    view_mode: ViewMode
}

impl Default for AppState {
    fn default() -> Self {
        Self {
            files: vec![
                FileItem { name: "file1".to_string(), size: 100},
                FileItem { name: "file2".to_string(), size: 200},
            ],
            view_mode: ViewMode::Ready,
        }
    }    
}

impl AppState {
    fn update(&mut self, mode: ViewMode) {
        match mode {
            ViewMode::Loading => {
                // Handle loading state
            }
            ViewMode::Ready => {
                // Handle ready state
            }
        }
    }



    fn view(&self) -> Column<'_, ViewMode> {

        //column!(text("Loading..."))

        match self.view_mode {
            ViewMode::Loading => column![text("Loading...")],
            ViewMode::Ready => {
                let children = self.files.iter()
                    .map(|item: &FileItem| text(format!("{} - {} bytes", item.name, item.size)).into())
                    .collect::<Vec<Element<'_, ViewMode>>>();

                Column::with_children(children)
            }
        }
    }
}


/* FileListWidget removed: building children directly in `AppState::view` avoids lifetime/temp issues */

fn main() -> iced::Result { // Result<(), slint::PlatformError>  {
    // let app: App = App::new()?;
    // let files = get_files();
    // let newfiles = files.iter().map(|e| UiFileItem { name: e.name.clone().into(), size: e.size as i32});
    // let test = ModelRc::new(VecModel::from(newfiles.collect::<Vec<UiFileItem>>()));

    // app.set_files(test);
    // app.run()?;


    

    println!("Hello, world!");
    //Ok(())
    
    iced::run(AppState::update, AppState::view)
}

fn get_files() -> Vec<FileItem> {
    vec![
        FileItem { name: "file1".to_string(), size: 100},
        FileItem { name: "file2".to_string(), size: 100},
    ]
}



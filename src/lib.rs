/*
 * Copyright (c) 2017 Boucher, Antoni <bouanto@zoho.com>
 *
 * Permission is hereby granted, free of charge, to any person obtaining a copy of
 * this software and associated documentation files (the "Software"), to deal in
 * the Software without restriction, including without limitation the rights to
 * use, copy, modify, merge, publish, distribute, sublicense, and/or sell copies of
 * the Software, and to permit persons to whom the Software is furnished to do so,
 * subject to the following conditions:
 *
 * The above copyright notice and this permission notice shall be included in all
 * copies or substantial portions of the Software.
 *
 * THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
 * IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY, FITNESS
 * FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE AUTHORS OR
 * COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER LIABILITY, WHETHER
 * IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM, OUT OF OR IN
 * CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE SOFTWARE.
 */

/*
 * TODO: integrate the tokio main loop into the GTK+ main loop so that the example nested-loop
 * works.
 * TODO: chat client/server example.
 *
 * TODO: try tk-easyloop in another branch.
 *
 * TODO: err if trying to use the SimpleMsg custom derive on stable.
 * TODO: Use two update functions (one for errors, one for success/normal behavior).
 *
 * TODO: add Cargo travis badge.
 * TODO: use macros 2.0 instead for the:
 * * view: to create the dependencies between the view items and the model.
 * * model: to add boolean fields in an inner struct specifying which parts of the view to update
 * *        after the update.
 * * update: to set the boolean fields to true depending on which parts of the model was updated.
 * * create default values for gtk widgets (like Label::new(None)).
 * * create attributes for constructor gtk widgets (like orientation for Box::new(orientation)).
 * TODO: optionnaly multi-threaded.
 * TODO: convert GTK+ callback to Stream (does not seem worth it, nor convenient since it will
 * still need to use USFC for the callback method).
 */

#![feature(conservative_impl_trait)]

extern crate futures;
extern crate gtk;
#[macro_use]
extern crate log;
extern crate relm_core;

mod macros;
mod stream;
mod widget;

use std::error;
use std::fmt::{self, Display, Formatter};
use std::io;
use std::time::SystemTime;

use futures::{Future, Stream};
use gtk::{ContainerExt, IsA, Object, WidgetExt};
use relm_core::Core;
pub use relm_core::{EventStream, Handle, QuitFuture};

pub use self::Error::*;
use self::stream::ToStream;
pub use self::widget::*;

pub struct Component<M, W> {
    stream: EventStream<M>,
    widget: W,
}

impl<M: Clone, W> Component<M, W> {
    pub fn stream(&self) -> &EventStream<M> {
        &self.stream
    }
}

#[derive(Debug)]
pub enum Error {
    GtkInit,
    Io(io::Error),
}

impl Display for Error {
    fn fmt(&self, formatter: &mut Formatter) -> fmt::Result {
        match *self {
            GtkInit => write!(formatter, "Cannot init GTK+"),
            Io(ref error) => write!(formatter, "IO error: {}", error),
        }
    }
}

impl error::Error for Error {
    fn description(&self) -> &str {
        match *self {
            GtkInit => "Cannot init GTK+",
            Io(ref error) => error.description(),
        }
    }

    fn cause(&self) -> Option<&error::Error> {
        match *self {
            GtkInit => None,
            Io(ref error) => Some(error),
        }
    }
}

impl From<io::Error> for Error {
    fn from(error: io::Error) -> Error {
        Io(error)
    }
}

impl From<()> for Error {
    fn from((): ()) -> Error {
        GtkInit
    }
}

pub struct Relm<M: Clone + DisplayVariant> {
    handle: Handle,
    stream: EventStream<M>,
}

impl<M: Clone + DisplayVariant + 'static> Relm<M> {
    pub fn connect<C, S, T>(&self, to_stream: T, callback: C) -> impl Future<Item=(), Error=()>
        where C: Fn(S::Item) -> M + 'static,
              S: Stream + 'static,
              T: ToStream<S, Item=S::Item, Error=S::Error> + 'static,
    {
        let event_stream = self.stream.clone();
        let stream = to_stream.to_stream();
        stream.map_err(|_| ()).for_each(move |result| {
            event_stream.emit(callback(result));
            Ok::<(), ()>(())
        }
            // TODO: handle errors.
            .map_err(|_| ()))
    }

    pub fn connect_exec<C, S, T>(&self, to_stream: T, callback: C)
        where C: Fn(S::Item) -> M + 'static,
              S: Stream + 'static,
              T: ToStream<S, Item=S::Item, Error=S::Error> + 'static,
    {
        self.exec(self.connect(to_stream, callback));
    }

    pub fn exec<F: Future<Item=(), Error=()> + 'static>(&self, future: F) {
        self.handle.spawn(future);
    }

    pub fn handle(&self) -> &Handle {
        &self.handle
    }

    pub fn run<D>() -> Result<(), Error>
        where D: Widget<M> + 'static,
    {
        gtk::init()?;

        let mut core = Core::new()?;

        let handle = core.handle();
        create_widget::<D, M>(&handle);

        core.run();
        Ok(())
    }

    // TODO: delete this method when the connect macros are no longer required.
    pub fn stream(&self) -> &EventStream<M> {
        &self.stream
    }
}

fn create_widget<D, M>(handle: &Handle) -> Component<M, D::Container>
    where D: Widget<M> + 'static,
          M: Clone + DisplayVariant + 'static,
{
    let stream = EventStream::new();

    let relm = Relm {
        handle: handle.clone(),
        stream: stream.clone(),
    };
    let mut widget = D::new(relm);

    let container = widget.container().clone();

    let event_future = {
        stream.clone().for_each(move |event| {
            if cfg!(debug_assertions) {
                let time = SystemTime::now();
                let debug = event.display_variant();
                let debug =
                    if debug.len() > 100 {
                        format!("{}…", &debug[..100])
                    }
                    else {
                        debug.to_string()
                    };
                widget.update(event);
                if let Ok(duration) = time.elapsed() {
                    let ms = duration.subsec_nanos() as u64 / 1_000_000 + duration.as_secs() * 1000;
                    if ms >= 200 {
                        // TODO: only show the message Variant because the value can be big.
                        warn!("The update function was slow to execute for message {}: {}ms", debug, ms);
                    }
                }
            }
            else {
                widget.update(event)
            }
            Ok(())
        })
    };
    handle.spawn(event_future);

    Component {
        stream: stream,
        widget: container,
    }
}

pub trait ContainerWidget
    where Self: ContainerExt
{
    fn add_widget<D, M>(&self, handle: &Handle) -> Component<M, D::Container>
        where D: Widget<M> + 'static,
              D::Container: IsA<Object> + WidgetExt,
              M: Clone + DisplayVariant + 'static,
    {
        let component = create_widget::<D, M>(handle);
        self.add(&component.widget);
        component.widget.show_all();
        component
    }

    fn remove_widget<M, W>(&self, widget: Component<M, W>)
        where W: IsA<gtk::Widget>,
    {
        self.remove(&widget.widget);
    }
}

impl<W: ContainerExt> ContainerWidget for W { }

pub trait DisplayVariant {
    fn display_variant(&self) -> &'static str;
}

#[cfg(test)]
#[derive(Debug)]
pub struct CounterAddOp(pub i32);

#[cfg(test)]
impl Apply<i32, ()> for CounterAddOp {
    fn apply_first(&mut self, first: &mut i32, _: &i32, _: &mut ()) {
        *first += self.0;
    }

    fn apply_second(self, _: &i32, second: &mut i32, _: &mut ()) {
        *second += self.0;
    }
}
